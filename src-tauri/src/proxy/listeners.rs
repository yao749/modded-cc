//! 代理 listener 的绑定. `ProxyConfig` 是「建 listener / router 时读取的那几项设置」的快照,
//! 与当前设置不等 = 有未生效的改动. 这里只 bind 不 serve, serve / 关停在 `controller.rs`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde::Serialize;
use tokio::net::TcpListener;

use crate::error::{AppError, AppResult};
use crate::settings::model::{ProxyMode, Settings};

/// 需要重启代理服务才生效的 6 项设置. `tls_extra_sans` 刻意不在内:
/// 它经「重新生成证书」热替换生效, 与重建 listener 无关.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProxyConfig {
    pub proxy_mode: ProxyMode,
    pub proxy_port: u16,
    pub https_port: u16,
    pub listen_all: bool,
    pub https_enable_h2: bool,
    pub max_request_body_mb: u32,
}

impl ProxyConfig {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            proxy_mode: s.proxy_mode,
            proxy_port: s.proxy_port,
            https_port: s.https_port,
            listen_all: s.listen_all,
            https_enable_h2: s.https_enable_h2,
            max_request_body_mb: s.max_request_body_mb,
        }
    }

    pub fn host(&self) -> IpAddr {
        if self.listen_all {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        }
    }

    /// 用户手改 settings.json 填 0 时按 1 MiB 兜底, 不能把代理配成完全收不了请求.
    pub fn body_limit_bytes(&self) -> usize {
        self.max_request_body_mb.max(1) as usize * 1024 * 1024
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BindOpts {
    /// 首选端口被占时最多尝试几个端口 (含首选端口本身).
    pub max_port_tries: u16,
    /// 端口在 `releasing` 里时, 遇到 AddrInUse 最多等多久再顺延.
    pub release_wait: Duration,
}

impl BindOpts {
    pub const DEFAULT: Self = Self {
        max_port_tries: 100,
        release_wait: Duration::from_secs(2),
    };

    /// 回滚用: 钉死在旧的实际端口, 不顺延 —— 客户端配的就是那个端口.
    pub fn pinned(self) -> Self {
        Self {
            max_port_tries: 1,
            ..self
        }
    }
}

const RELEASE_POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BoundPorts {
    pub http: Option<u16>,
    pub https: Option<u16>,
}

impl BoundPorts {
    pub fn list(&self) -> Vec<u16> {
        self.http.into_iter().chain(self.https).collect()
    }
}

/// 要绑哪些端口. `None` = 该协议不启用.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenSpec {
    pub host: IpAddr,
    pub http_port: Option<u16>,
    pub https_port: Option<u16>,
}

impl ListenSpec {
    pub fn from_config(c: &ProxyConfig) -> Self {
        Self {
            host: c.host(),
            http_port: c.proxy_mode.includes_http().then_some(c.proxy_port),
            https_port: c.proxy_mode.includes_https().then_some(c.https_port),
        }
    }
}

/// 已绑定但尚未开始 serve 的 listener.
#[derive(Debug)]
pub struct Bound {
    pub host: IpAddr,
    pub http: Option<(TcpListener, u16)>,
    pub https: Option<(TcpListener, u16)>,
}

impl Bound {
    pub fn ports(&self) -> BoundPorts {
        BoundPorts {
            http: self.http.as_ref().map(|(_, p)| *p),
            https: self.https.as_ref().map(|(_, p)| *p),
        }
    }
}

/// 从 `start_port` 起找一个能绑的端口, 被占则 +1, 最多 `opts.max_port_tries` 个.
///
/// `releasing` 是旧实例刚放手、正在异步释放的端口: 遇到 AddrInUse 时先在 `release_wait`
/// 内重试, 仍占用才顺延. 否则新旧端口相同时 (只改监听地址 / 请求体上限 / h2) 会撞上
/// 自己的旧 listener, 被悄悄顺延到下一个端口, 客户端全部连不上.
pub async fn bind_with_fallback(
    host: IpAddr,
    start_port: u16,
    releasing: &[u16],
    opts: BindOpts,
) -> AppResult<(TcpListener, u16)> {
    let tries = opts.max_port_tries.max(1);
    let mut port = start_port;
    let mut last = start_port;
    for _ in 0..tries {
        last = port;
        match bind_one(host, port, releasing.contains(&port), opts.release_wait).await {
            Ok(listener) => return Ok((listener, port)),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => match port.checked_add(1) {
                Some(next) => port = next,
                None => break,
            },
            Err(e) => return Err(AppError::internal(format!("端口 {port} 无法绑定: {e}"))),
        }
    }
    Err(AppError::internal(if last == start_port {
        format!("端口 {start_port} 已被占用")
    } else {
        format!("端口 {start_port}–{last} 均已被占用")
    }))
}

async fn bind_one(
    host: IpAddr,
    port: u16,
    releasing: bool,
    wait: Duration,
) -> std::io::Result<TcpListener> {
    let addr = SocketAddr::new(host, port);
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match TcpListener::bind(addr).await {
            Err(e)
                if releasing
                    && e.kind() == std::io::ErrorKind::AddrInUse
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(RELEASE_POLL).await;
            }
            other => return other,
        }
    }
}

/// 把 `spec` 里启用的每一路都绑上, 任一路失败即整体失败. 失败时已绑上的那路随 `?`
/// 一起 drop, 端口立即释放 —— 所以调用方拿到 Err 时不会有「半个新实例」占着端口.
pub async fn bind_all(spec: &ListenSpec, releasing: &[u16], opts: BindOpts) -> AppResult<Bound> {
    if spec.http_port.is_none() && spec.https_port.is_none() {
        return Err(AppError::internal("proxy_mode 没有任何 listener 被启用"));
    }
    let http = match spec.http_port {
        Some(p) => Some(bind_with_fallback(spec.host, p, releasing, opts).await?),
        None => None,
    };
    // HTTP 刚占下的端口是新实例自己的, 对 HTTPS 而言不算「正在释放」, 撞上直接顺延.
    let http_port = http.as_ref().map(|(_, p)| *p);
    let releasing_for_https: Vec<u16> = releasing
        .iter()
        .copied()
        .filter(|p| Some(*p) != http_port)
        .collect();
    let https = match spec.https_port {
        Some(p) => Some(bind_with_fallback(spec.host, p, &releasing_for_https, opts).await?),
        None => None,
    };
    Ok(Bound {
        host: spec.host,
        http,
        https,
    })
}

#[cfg(test)]
pub(crate) fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    fn cfg() -> ProxyConfig {
        ProxyConfig::from_settings(&Settings::default())
    }

    #[test]
    fn from_settings_copies_the_six_fields() {
        let mut s = Settings::default();
        s.proxy_mode = ProxyMode::Both;
        s.proxy_port = 1111;
        s.https_port = 2222;
        s.listen_all = true;
        s.https_enable_h2 = false;
        s.max_request_body_mb = 64;
        assert_eq!(
            ProxyConfig::from_settings(&s),
            ProxyConfig {
                proxy_mode: ProxyMode::Both,
                proxy_port: 1111,
                https_port: 2222,
                listen_all: true,
                https_enable_h2: false,
                max_request_body_mb: 64,
            }
        );
    }

    #[test]
    fn unrelated_settings_do_not_change_config() {
        let base = cfg();
        let mut s = Settings::default();
        s.tls_extra_sans = vec!["192.168.1.5".into()];
        s.auth_enabled = !s.auth_enabled;
        s.preferred_language = "ja".into();
        s.web_ui_enabled = !s.web_ui_enabled;
        assert_eq!(ProxyConfig::from_settings(&s), base);
    }

    #[test]
    fn host_follows_listen_all() {
        let mut c = cfg();
        c.listen_all = false;
        assert_eq!(c.host(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        c.listen_all = true;
        assert_eq!(c.host(), IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }

    #[test]
    fn body_limit_clamps_zero_to_one_mib() {
        let mut c = cfg();
        c.max_request_body_mb = 0;
        assert_eq!(c.body_limit_bytes(), 1024 * 1024);
        c.max_request_body_mb = 32;
        assert_eq!(c.body_limit_bytes(), 32 * 1024 * 1024);
    }

    #[test]
    fn listen_spec_follows_mode() {
        let mut c = cfg();
        c.proxy_port = 10;
        c.https_port = 20;
        c.proxy_mode = ProxyMode::Http;
        assert_eq!(ListenSpec::from_config(&c).http_port, Some(10));
        assert_eq!(ListenSpec::from_config(&c).https_port, None);
        c.proxy_mode = ProxyMode::Https;
        assert_eq!(ListenSpec::from_config(&c).http_port, None);
        assert_eq!(ListenSpec::from_config(&c).https_port, Some(20));
        c.proxy_mode = ProxyMode::Both;
        assert_eq!(ListenSpec::from_config(&c).http_port, Some(10));
        assert_eq!(ListenSpec::from_config(&c).https_port, Some(20));
    }

    #[tokio::test]
    async fn taken_port_falls_forward() {
        let p = free_port();
        let _blocker = std::net::TcpListener::bind((LOCAL, p)).unwrap();
        let (_l, got) = bind_with_fallback(LOCAL, p, &[], BindOpts::DEFAULT).await.unwrap();
        assert!(got > p, "应顺延到 {p} 之后, 实际 {got}");
    }

    #[tokio::test]
    async fn releasing_port_is_waited_for_instead_of_skipped() {
        let p = free_port();
        let blocker = std::net::TcpListener::bind((LOCAL, p)).unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            drop(blocker);
        });
        let (_l, got) = bind_with_fallback(LOCAL, p, &[p], BindOpts::DEFAULT).await.unwrap();
        assert_eq!(got, p, "自己正在释放的端口应等它放手, 不能顺延");
    }

    #[tokio::test]
    async fn pinned_does_not_fall_forward() {
        let p = free_port();
        let _blocker = std::net::TcpListener::bind((LOCAL, p)).unwrap();
        let opts = BindOpts {
            release_wait: Duration::from_millis(50),
            ..BindOpts::DEFAULT
        }
        .pinned();
        let err = bind_with_fallback(LOCAL, p, &[p], opts).await.unwrap_err();
        assert!(err.to_string().contains(&p.to_string()), "错误里应带端口号: {err}");
    }

    #[tokio::test]
    async fn bind_all_releases_http_when_https_fails() {
        let p_http = free_port();
        let p_https = free_port();
        let _blocker = std::net::TcpListener::bind((LOCAL, p_https)).unwrap();
        let spec = ListenSpec {
            host: LOCAL,
            http_port: Some(p_http),
            https_port: Some(p_https),
        };
        assert!(bind_all(&spec, &[], BindOpts::DEFAULT.pinned()).await.is_err());
        // HTTP 那路绑上后又随失败一起被丢掉, 端口必须立即可用
        std::net::TcpListener::bind((LOCAL, p_http)).expect("HTTP 端口应已释放");
    }

    #[tokio::test]
    async fn bind_all_rejects_empty_spec() {
        let spec = ListenSpec {
            host: LOCAL,
            http_port: None,
            https_port: None,
        };
        assert!(bind_all(&spec, &[], BindOpts::DEFAULT).await.is_err());
    }
}
