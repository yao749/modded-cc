//! 代理服务的生命周期: 按当前设置 (重新) 绑定 listener, 优雅关停旧实例, 失败回滚.
//!
//! 与 AppState / Tauri 解耦: 取设置、造 TLS 配置、建 router、绑定后的副作用都经 `ProxyHooks`,
//! 生产实现是 `server.rs::AppHooks`, 测试用假钩子跑真实 TCP.
//!
//! 重启顺序是「先停旧的、再绑新的」: 反过来的话新旧端口相同时新 listener 会撞上自己的旧
//! listener, 被 +1 逻辑悄悄顺延. 旧实例的在途请求继续跑完 (它与新实例共享同一份 AppState),
//! 重启不等它们排空.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::Duration;

use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use tokio::sync::{oneshot, Mutex};
use tracing::{info, warn};

use crate::error::{AppError, AppResult};
use crate::proxy::listeners::{bind_all, BindOpts, Bound, BoundPorts, ListenSpec, ProxyConfig};

/// HTTPS 路的排空上限 (与 http_client 的 600s 空闲超时同值, 但那是空闲上限不是总上限,
/// 持续出数据的长流可以比它活得久, 届时被这里截断). HTTP 路 axum 不支持上限: 普通请求
/// 跑完或客户端断开即结束; 无尽的事件流 (/ui/api/events) 则靠转发 `proxy_restarted`
/// 之后主动结束来让旧实例排空.
const HTTPS_DRAIN: Duration = Duration::from_secs(600);

pub trait ProxyHooks: Clone + Send + Sync + 'static {
    /// 按当前设置算出的目标配置.
    fn desired_config(&self) -> impl Future<Output = ProxyConfig> + Send;
    /// 从磁盘上的证书构建 rustls 配置 (CA / 叶证书缺失时生成).
    fn tls_config(
        &self,
        enable_h2: bool,
    ) -> impl Future<Output = AppResult<Arc<rustls::ServerConfig>>> + Send;
    fn build_router(&self, cfg: &ProxyConfig) -> Router;
    /// 绑定成功 (Some) 或代理停止 (None) 之后调用. 在 controller 的锁内调用, 不得回调 controller.
    fn on_bound(&self, ports: Option<BoundPorts>) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// 正在运行的实例的生效配置; None = 代理未运行.
    pub applied: Option<ProxyConfig>,
    /// 最近一次启动 / 重启 / 运行中崩溃的原因; 成功应用新配置后清空.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartOutcome {
    /// 新配置已生效 (端口可能被顺延, 由调用方比较 applied 与实际端口得知).
    Applied,
    /// 新配置绑定失败, 已回到旧的实际地址运行.
    RolledBack(String),
    /// 新配置失败且回滚也失败 (或本来就没有旧实例), 代理未运行.
    Stopped(String),
    /// 还没碰 listener 就失败了 (如 TLS 配置构建失败), 旧实例原样运行.
    Failed(String),
}

struct Serving {
    ports: BoundPorts,
    /// 每路一个; send 或 drop 都会触发该路的优雅关停.
    stops: Vec<oneshot::Sender<()>>,
    /// 仅 HTTPS 路启用时有值. 可 clone (内部是 Arc), 回滚时原样复用, 热替换时换内容.
    tls: Option<RustlsConfig>,
}

impl Serving {
    fn shutdown(self) {
        for stop in self.stops {
            let _ = stop.send(());
        }
    }
}

struct Running {
    /// 实例代次. listener 任务退出时据此判断「是被我们关掉的旧实例」还是「当前实例意外退出」.
    gen: u64,
    applied: ProxyConfig,
    serving: Serving,
}

pub struct ProxyController {
    opts: BindOpts,
    /// 串行化 restart / reload_tls / 意外退出处理.
    running: Mutex<Option<Running>>,
    /// 给状态查询用的副本, 不碰上面那把锁, 一次长重启不会卡住界面的 5 秒轮询.
    snapshot: StdRwLock<Snapshot>,
    next_gen: AtomicU64,
}

impl ProxyController {
    pub fn new(opts: BindOpts) -> Self {
        Self {
            opts,
            running: Mutex::new(None),
            snapshot: StdRwLock::new(Snapshot::default()),
            next_gen: AtomicU64::new(1),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// 代理在运行且生效配置与 `desired` 不同. 未运行时恒 false (界面改为提示「未运行」).
    pub fn restart_pending(&self, desired: &ProxyConfig) -> bool {
        self.snapshot().applied.is_some_and(|a| a != *desired)
    }

    fn set_snapshot(&self, applied: Option<ProxyConfig>, last_error: Option<String>) {
        *self.snapshot.write().unwrap_or_else(|p| p.into_inner()) = Snapshot {
            applied,
            last_error,
        };
    }

    /// 按当前设置 (重新) 启动代理. 首次启动也走这里 (此时没有旧实例可回滚).
    ///
    /// 整个切换跑在独立任务里: 调用方 (网页界面 / TUI 的请求) 中途断开会 drop 这个 future,
    /// 若切换跑在调用方的 future 里, 在「旧实例已关、新实例未起」时被取消, 快照会停在
    /// 「运行中」而实际没有任何 listener.
    pub async fn restart<H: ProxyHooks>(self: &Arc<Self>, hooks: H) -> RestartOutcome {
        let ctl = Arc::clone(self);
        match tokio::spawn(async move { ctl.restart_inner(hooks).await }).await {
            Ok(outcome) => outcome,
            Err(e) => RestartOutcome::Stopped(format!("重启任务异常结束: {e}")),
        }
    }

    async fn restart_inner<H: ProxyHooks>(self: Arc<Self>, hooks: H) -> RestartOutcome {
        let mut running = self.running.lock().await;
        let desired = hooks.desired_config().await;

        let tls = if desired.proxy_mode.includes_https() {
            match hooks.tls_config(desired.https_enable_h2).await {
                Ok(cfg) => Some(RustlsConfig::from_config(cfg)),
                Err(e) => {
                    let msg = plain(&e);
                    warn!(error = %msg, "代理重启在动 listener 之前失败, 旧实例保持运行");
                    if running.is_none() {
                        // 没有旧实例可保留: 这是「已停止」而不是「旧实例仍在运行」
                        self.set_snapshot(None, Some(msg.clone()));
                        return RestartOutcome::Stopped(msg);
                    }
                    return RestartOutcome::Failed(msg);
                }
            }
        } else {
            None
        };

        let old = running.take();
        let releasing = old
            .as_ref()
            .map(|r| r.serving.ports.list())
            .unwrap_or_default();
        let rollback = old.map(|r| {
            let plan = Rollback {
                applied: r.applied,
                ports: r.serving.ports,
                tls: r.serving.tls.clone(),
            };
            r.serving.shutdown();
            plan
        });

        let spec = ListenSpec::from_config(&desired);
        let err = match self
            .bring_up(desired, &spec, &releasing, self.opts, tls, &hooks)
            .await
        {
            Ok(r) => {
                let ports = r.serving.ports;
                *running = Some(r);
                self.set_snapshot(Some(desired), None);
                hooks.on_bound(Some(ports)).await;
                info!(?ports, ?desired, "代理已按新配置运行");
                return RestartOutcome::Applied;
            }
            Err(e) => plain(&e),
        };
        warn!(error = %err, "新配置绑定失败");

        let Some(rb) = rollback else {
            self.set_snapshot(None, Some(err.clone()));
            hooks.on_bound(None).await;
            return RestartOutcome::Stopped(err);
        };
        // 回到旧的**实际**端口 (当初若被顺延过, 客户端配的就是顺延后的那个), 不再顺延.
        let spec = ListenSpec {
            host: rb.applied.host(),
            http_port: rb.ports.http,
            https_port: rb.ports.https,
        };
        match self
            .bring_up(rb.applied, &spec, &releasing, self.opts.pinned(), rb.tls, &hooks)
            .await
        {
            Ok(r) => {
                let ports = r.serving.ports;
                *running = Some(r);
                self.set_snapshot(Some(rb.applied), Some(err.clone()));
                hooks.on_bound(Some(ports)).await;
                info!(?ports, "已回滚到原配置");
                RestartOutcome::RolledBack(err)
            }
            Err(e2) => {
                let msg = format!("{err}; 回滚也失败: {}", plain(&e2));
                warn!(error = %msg, "代理已停止");
                self.set_snapshot(None, Some(msg.clone()));
                hooks.on_bound(None).await;
                RestartOutcome::Stopped(msg)
            }
        }
    }

    async fn bring_up<H: ProxyHooks>(
        self: &Arc<Self>,
        applied: ProxyConfig,
        spec: &ListenSpec,
        releasing: &[u16],
        opts: BindOpts,
        tls: Option<RustlsConfig>,
        hooks: &H,
    ) -> AppResult<Running> {
        let bound = bind_all(spec, releasing, opts).await?;
        self.launch(applied, bound, tls, hooks)
    }

    /// 开始 serve. HTTPS 路的转换 (可能失败) 放在任何一路开始 serve 之前,
    /// 保证返回 Err 时一个请求都没接过、listener 随 `bound` 一起释放.
    fn launch<H: ProxyHooks>(
        self: &Arc<Self>,
        applied: ProxyConfig,
        bound: Bound,
        tls: Option<RustlsConfig>,
        hooks: &H,
    ) -> AppResult<Running> {
        let gen = self.next_gen.fetch_add(1, Ordering::Relaxed);
        let ports = bound.ports();
        let https_server = match bound.https {
            Some((listener, _)) => {
                let tls = tls
                    .clone()
                    .ok_or_else(|| AppError::internal("HTTPS 模式但 TLS 配置未初始化"))?;
                let std_listener = listener.into_std().map_err(AppError::Io)?;
                std_listener.set_nonblocking(true).map_err(AppError::Io)?;
                Some(axum_server::from_tcp_rustls(std_listener, tls).map_err(AppError::Io)?)
            }
            None => None,
        };

        let router = hooks.build_router(&applied);
        let mut stops = Vec::new();

        if let Some((listener, _)) = bound.http {
            let (tx, rx) = oneshot::channel::<()>();
            stops.push(tx);
            let svc = router
                .clone()
                .into_make_service_with_connect_info::<SocketAddr>();
            let fut = async move {
                axum::serve(listener, svc)
                    .with_graceful_shutdown(async move {
                        let _ = rx.await;
                    })
                    .await
                    .map_err(|e| format!("HTTP listener: {e}"))
            };
            self.watch(gen, "http", fut, hooks.clone());
        }

        if let Some(server) = https_server {
            let (tx, rx) = oneshot::channel::<()>();
            stops.push(tx);
            let handle: axum_server::Handle<SocketAddr> = axum_server::Handle::default();
            let server = server.handle(handle.clone());
            let svc = router.into_make_service_with_connect_info::<SocketAddr>();
            let fut = async move {
                let serve = server.serve(svc);
                tokio::pin!(serve);
                tokio::select! {
                    r = &mut serve => return r.map_err(|e| format!("HTTPS listener: {e}")),
                    _ = rx => {}
                }
                // 关停信号是粘性的 (NotifyOnce), 即便 serve 还没被首次 poll 也不会丢.
                handle.graceful_shutdown(Some(HTTPS_DRAIN));
                (&mut serve)
                    .await
                    .map_err(|e| format!("HTTPS listener: {e}"))
            };
            self.watch(gen, "https", fut, hooks.clone());
        }

        Ok(Running {
            gen,
            applied,
            serving: Serving { ports, stops, tls },
        })
    }

    fn watch<H: ProxyHooks>(
        self: &Arc<Self>,
        gen: u64,
        which: &'static str,
        fut: impl Future<Output = Result<(), String>> + Send + 'static,
        hooks: H,
    ) {
        let ctl = Arc::clone(self);
        tokio::spawn(async move {
            let reason = match fut.await {
                Ok(()) => format!("{which} listener 已退出"),
                Err(e) => e,
            };
            ctl.handle_exit(gen, which, reason, hooks).await;
        });
    }

    /// listener 任务结束时调用. 被我们主动关掉的旧实例 (代次不是当前的) 只记日志;
    /// 当前实例意外退出则整体停止 (另一路也关掉), 界面显示「未运行」与原因.
    async fn handle_exit<H: ProxyHooks>(&self, gen: u64, which: &str, reason: String, hooks: H) {
        let mut running = self.running.lock().await;
        if running.as_ref().map(|r| r.gen) != Some(gen) {
            info!(gen, which, "旧代理实例已排空退出");
            return;
        }
        warn!(gen, which, %reason, "代理 listener 意外退出, 代理停止");
        if let Some(r) = running.take() {
            r.serving.shutdown();
        }
        self.set_snapshot(None, Some(reason));
        hooks.on_bound(None).await;
    }

    /// 重新生成叶证书后调用: 运行中含 HTTPS 就热替换 —— 新握手立即用新证书, 已建立的
    /// TLS 连接不受影响, 不重绑端口. ALPN 取生效快照里的 h2, 不取设置里的.
    /// 返回是否真的替换了.
    pub async fn reload_tls<H: ProxyHooks>(&self, hooks: &H) -> AppResult<bool> {
        let running = self.running.lock().await;
        let Some(r) = running.as_ref() else {
            return Ok(false);
        };
        let Some(tls) = r.serving.tls.as_ref() else {
            return Ok(false);
        };
        let cfg = hooks.tls_config(r.applied.https_enable_h2).await?;
        tls.reload_from_config(cfg);
        Ok(true)
    }
}

struct Rollback {
    applied: ProxyConfig,
    ports: BoundPorts,
    tls: Option<RustlsConfig>,
}

/// 给界面看的错误文字: 内部错误去掉「内部错误: 」前缀, 其余用原本的 Display.
fn plain(e: &AppError) -> String {
    match e {
        AppError::Internal(m) => m.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::listeners::free_port;
    use crate::settings::model::ProxyMode;
    use axum::routing::get;
    use std::net::{IpAddr, Ipv4Addr};

    const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    #[derive(Clone)]
    struct TestHooks {
        desired: Arc<std::sync::Mutex<ProxyConfig>>,
        tls_dir: Arc<tempfile::TempDir>,
        bound: Arc<std::sync::Mutex<Vec<Option<BoundPorts>>>>,
        tls_fail: Arc<std::sync::atomic::AtomicBool>,
    }

    impl TestHooks {
        fn new(cfg: ProxyConfig) -> Self {
            Self {
                desired: Arc::new(std::sync::Mutex::new(cfg)),
                tls_dir: Arc::new(tempfile::tempdir().unwrap()),
                bound: Arc::new(std::sync::Mutex::new(Vec::new())),
                tls_fail: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }
        }
        fn set(&self, cfg: ProxyConfig) {
            *self.desired.lock().unwrap() = cfg;
        }
        fn last_bound(&self) -> Option<BoundPorts> {
            *self.bound.lock().unwrap().last().expect("on_bound 应至少被调用一次")
        }
    }

    impl ProxyHooks for TestHooks {
        async fn desired_config(&self) -> ProxyConfig {
            *self.desired.lock().unwrap()
        }
        async fn tls_config(&self, enable_h2: bool) -> AppResult<Arc<rustls::ServerConfig>> {
            if self.tls_fail.load(Ordering::SeqCst) {
                return Err(AppError::internal("证书不可用"));
            }
            crate::tls::load_or_init_server_config(self.tls_dir.path(), &[], enable_h2).await
        }
        fn build_router(&self, cfg: &ProxyConfig) -> Router {
            Router::new()
                .route("/ping", get(|| async { "pong" }))
                .route(
                    "/slow",
                    get(|| async {
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        "done"
                    }),
                )
                .layer(axum::extract::DefaultBodyLimit::max(cfg.body_limit_bytes()))
        }
        async fn on_bound(&self, ports: Option<BoundPorts>) {
            self.bound.lock().unwrap().push(ports);
        }
    }

    fn http(port: u16) -> ProxyConfig {
        ProxyConfig {
            proxy_mode: ProxyMode::Http,
            proxy_port: port,
            https_port: 0,
            listen_all: false,
            https_enable_h2: true,
            max_request_body_mb: 32,
        }
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    async fn get_text(port: u16, path: &str) -> reqwest::Result<String> {
        client()
            .get(format!("http://127.0.0.1:{port}{path}"))
            .send()
            .await?
            .text()
            .await
    }

    fn ctl(opts: BindOpts) -> Arc<ProxyController> {
        Arc::new(ProxyController::new(opts))
    }

    fn fast_pinned() -> BindOpts {
        BindOpts {
            release_wait: Duration::from_millis(200),
            ..BindOpts::DEFAULT
        }
        .pinned()
    }

    async fn current_gen(c: &ProxyController) -> Option<u64> {
        c.running.lock().await.as_ref().map(|r| r.gen)
    }

    #[tokio::test]
    async fn first_start_applies_and_serves() {
        let p = free_port();
        let hooks = TestHooks::new(http(p));
        let c = ctl(BindOpts::DEFAULT);
        assert_eq!(c.restart(hooks.clone()).await, RestartOutcome::Applied);
        assert_eq!(hooks.last_bound(), Some(BoundPorts { http: Some(p), https: None }));
        assert_eq!(c.snapshot(), Snapshot { applied: Some(http(p)), last_error: None });
        assert_eq!(get_text(p, "/ping").await.unwrap(), "pong");
    }

    #[tokio::test]
    async fn same_port_rebind_does_not_fall_forward() {
        let p = free_port();
        let hooks = TestHooks::new(http(p));
        let c = ctl(BindOpts::DEFAULT);
        c.restart(hooks.clone()).await;
        let mut next = http(p);
        next.max_request_body_mb = 64;
        hooks.set(next);
        assert_eq!(c.restart(hooks.clone()).await, RestartOutcome::Applied);
        assert_eq!(hooks.last_bound().unwrap().http, Some(p), "同端口重绑不能被顺延");
        assert_eq!(c.snapshot().applied, Some(next));
        assert_eq!(get_text(p, "/ping").await.unwrap(), "pong");
    }

    #[tokio::test]
    async fn taken_port_falls_forward_on_restart() {
        let p1 = free_port();
        let hooks = TestHooks::new(http(p1));
        let c = ctl(BindOpts::DEFAULT);
        c.restart(hooks.clone()).await;
        let p2 = free_port();
        let _blocker = std::net::TcpListener::bind((LOCAL, p2)).unwrap();
        hooks.set(http(p2));
        assert_eq!(c.restart(hooks.clone()).await, RestartOutcome::Applied);
        let got = hooks.last_bound().unwrap().http.unwrap();
        assert!(got > p2, "应顺延到 {p2} 之后, 实际 {got}");
        assert_eq!(c.snapshot().applied.unwrap().proxy_port, p2, "applied 记首选端口");
    }

    #[tokio::test]
    async fn bind_failure_rolls_back_to_old_actual_port() {
        let p1 = free_port();
        let hooks = TestHooks::new(http(p1));
        let c = ctl(fast_pinned());
        assert_eq!(c.restart(hooks.clone()).await, RestartOutcome::Applied);
        let p2 = free_port();
        let _blocker = std::net::TcpListener::bind((LOCAL, p2)).unwrap();
        hooks.set(http(p2));
        let out = c.restart(hooks.clone()).await;
        assert!(matches!(out, RestartOutcome::RolledBack(_)), "{out:?}");
        let snap = c.snapshot();
        assert_eq!(snap.applied, Some(http(p1)));
        assert!(snap.last_error.is_some());
        assert_eq!(hooks.last_bound().unwrap().http, Some(p1));
        assert_eq!(get_text(p1, "/ping").await.unwrap(), "pong");
    }

    #[tokio::test]
    async fn both_mode_failure_releases_new_http_and_rolls_back() {
        let p1 = free_port();
        let hooks = TestHooks::new(http(p1));
        let c = ctl(fast_pinned());
        c.restart(hooks.clone()).await;
        let (p2, p3) = (free_port(), free_port());
        let _blocker = std::net::TcpListener::bind((LOCAL, p3)).unwrap();
        hooks.set(ProxyConfig {
            proxy_mode: ProxyMode::Both,
            proxy_port: p2,
            https_port: p3,
            ..http(p1)
        });
        let out = c.restart(hooks.clone()).await;
        assert!(matches!(out, RestartOutcome::RolledBack(_)), "{out:?}");
        std::net::TcpListener::bind((LOCAL, p2)).expect("新 HTTP 端口应已释放");
        assert_eq!(get_text(p1, "/ping").await.unwrap(), "pong");
    }

    #[tokio::test]
    async fn first_start_failure_is_stopped() {
        let p = free_port();
        let _blocker = std::net::TcpListener::bind((LOCAL, p)).unwrap();
        let hooks = TestHooks::new(http(p));
        let c = ctl(fast_pinned());
        let out = c.restart(hooks.clone()).await;
        assert!(matches!(out, RestartOutcome::Stopped(_)), "{out:?}");
        assert_eq!(c.snapshot().applied, None);
        assert!(c.snapshot().last_error.is_some());
        assert_eq!(hooks.last_bound(), None);
    }

    #[tokio::test]
    async fn in_flight_request_survives_restart() {
        let p1 = free_port();
        let hooks = TestHooks::new(http(p1));
        let c = ctl(BindOpts::DEFAULT);
        c.restart(hooks.clone()).await;
        let slow = tokio::spawn(get_text(p1, "/slow"));
        tokio::time::sleep(Duration::from_millis(100)).await;
        let p2 = free_port();
        hooks.set(http(p2));
        assert_eq!(c.restart(hooks.clone()).await, RestartOutcome::Applied);
        assert_eq!(slow.await.unwrap().unwrap(), "done", "在途请求应跑完");
        assert_eq!(get_text(p2, "/ping").await.unwrap(), "pong");
        assert!(get_text(p1, "/ping").await.is_err(), "旧端口应不再接受新连接");
    }

    #[tokio::test]
    async fn concurrent_restarts_serialize() {
        let p1 = free_port();
        let hooks = TestHooks::new(http(p1));
        let c = ctl(BindOpts::DEFAULT);
        c.restart(hooks.clone()).await;
        let p2 = free_port();
        hooks.set(http(p2));
        let (a, b) = tokio::join!(c.restart(hooks.clone()), c.restart(hooks.clone()));
        assert_eq!(a, RestartOutcome::Applied);
        assert_eq!(b, RestartOutcome::Applied);
        assert_eq!(hooks.last_bound().unwrap().http, Some(p2));
        assert_eq!(get_text(p2, "/ping").await.unwrap(), "pong");
    }

    #[tokio::test]
    async fn cancelled_restart_still_completes() {
        let p = free_port();
        let hooks = TestHooks::new(http(p));
        let c = ctl(BindOpts::DEFAULT);
        c.restart(hooks.clone()).await;
        let mut next = http(p);
        next.max_request_body_mb = 64;
        hooks.set(next);
        // 同端口重绑会先撞上还没释放的旧 listener, 停在等待释放的 sleep 里;
        // 此时调用方的 future 被丢弃 (模拟网页界面 / TUI 请求中途断开).
        let _ = tokio::time::timeout(Duration::ZERO, c.restart(hooks.clone())).await;
        let mut applied = false;
        for _ in 0..100 {
            if c.snapshot().applied == Some(next) {
                applied = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(applied, "调用方取消后切换应仍在后台完成, 快照: {:?}", c.snapshot());
        assert_eq!(get_text(p, "/ping").await.unwrap(), "pong");
    }

    #[tokio::test]
    async fn https_to_http_on_same_port_right_after_start() {
        // 守护: 刚启动的 HTTPS 实例 (服务任务可能还没被 poll 过) 也要能释放端口,
        // 否则同端口改回 HTTP 时新 listener 会被顺延.
        let p = free_port();
        let hooks = TestHooks::new(ProxyConfig {
            proxy_mode: ProxyMode::Https,
            https_port: p,
            ..http(0)
        });
        let c = ctl(BindOpts::DEFAULT);
        assert_eq!(c.restart(hooks.clone()).await, RestartOutcome::Applied);
        assert_eq!(hooks.last_bound().unwrap().https, Some(p));
        hooks.set(http(p));
        assert_eq!(c.restart(hooks.clone()).await, RestartOutcome::Applied);
        assert_eq!(hooks.last_bound().unwrap().http, Some(p));
    }

    #[tokio::test]
    async fn reload_tls_only_when_https_running() {
        let p = free_port();
        let hooks = TestHooks::new(http(p));
        let c = ctl(BindOpts::DEFAULT);
        assert!(!c.reload_tls(&hooks).await.unwrap(), "未运行时不做事");
        c.restart(hooks.clone()).await;
        assert!(!c.reload_tls(&hooks).await.unwrap(), "HTTP-only 时不做事");
        let q = free_port();
        hooks.set(ProxyConfig {
            proxy_mode: ProxyMode::Https,
            https_port: q,
            ..http(0)
        });
        c.restart(hooks.clone()).await;
        assert!(c.reload_tls(&hooks).await.unwrap());
    }

    #[tokio::test]
    async fn restart_pending_compares_against_applied() {
        let p = free_port();
        let hooks = TestHooks::new(http(p));
        let c = ctl(BindOpts::DEFAULT);
        assert!(!c.restart_pending(&http(p)), "未运行时恒 false");
        c.restart(hooks.clone()).await;
        assert!(!c.restart_pending(&http(p)));
        let mut other = http(p);
        other.https_enable_h2 = false;
        assert!(c.restart_pending(&other));
    }

    #[tokio::test]
    async fn unexpected_exit_of_current_generation_stops_proxy() {
        let p = free_port();
        let hooks = TestHooks::new(http(p));
        let c = ctl(BindOpts::DEFAULT);
        c.restart(hooks.clone()).await;
        let gen = current_gen(&c).await.unwrap();
        c.handle_exit(gen, "http", "boom".into(), hooks.clone()).await;
        assert_eq!(current_gen(&c).await, None);
        assert_eq!(c.snapshot(), Snapshot { applied: None, last_error: Some("boom".into()) });
        assert_eq!(hooks.last_bound(), None);
    }

    #[tokio::test]
    async fn exit_of_old_generation_is_ignored() {
        let p = free_port();
        let hooks = TestHooks::new(http(p));
        let c = ctl(BindOpts::DEFAULT);
        c.restart(hooks.clone()).await;
        let gen = current_gen(&c).await.unwrap();
        c.handle_exit(gen + 100, "http", "old".into(), hooks.clone()).await;
        assert_eq!(current_gen(&c).await, Some(gen));
        assert_eq!(c.snapshot().last_error, None);
    }

    #[tokio::test]
    async fn tls_failure_on_first_start_is_stopped() {
        let hooks = TestHooks::new(ProxyConfig {
            proxy_mode: ProxyMode::Https,
            https_port: free_port(),
            ..http(0)
        });
        hooks.tls_fail.store(true, Ordering::SeqCst);
        let c = ctl(BindOpts::DEFAULT);
        assert!(matches!(
            c.restart(hooks.clone()).await,
            RestartOutcome::Stopped(m) if m == "证书不可用"
        ));
        assert_eq!(
            c.snapshot(),
            Snapshot { applied: None, last_error: Some("证书不可用".into()) }
        );
    }

    #[tokio::test]
    async fn tls_failure_with_running_instance_keeps_it() {
        let p = free_port();
        let hooks = TestHooks::new(http(p));
        let c = ctl(BindOpts::DEFAULT);
        assert_eq!(c.restart(hooks.clone()).await, RestartOutcome::Applied);
        let before = c.snapshot();
        hooks.tls_fail.store(true, Ordering::SeqCst);
        hooks.set(ProxyConfig {
            proxy_mode: ProxyMode::Https,
            https_port: free_port(),
            ..http(0)
        });
        assert!(matches!(c.restart(hooks.clone()).await, RestartOutcome::Failed(_)));
        assert_eq!(c.snapshot(), before);
        assert_eq!(get_text(p, "/ping").await.unwrap(), "pong");
    }
}
