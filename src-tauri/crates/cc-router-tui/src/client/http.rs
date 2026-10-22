//! 调用桌面 app 的管理 API: `POST /ui/api/cmd/<name>` 与 `GET /ui/api/events`。
//!
//! 鉴权是「本机通行」: `x-ccr-local: <runtime.json 里的密钥>` + `x-ccr-ui` (CSRF 头)。
//! 后端对「本机通行不成立」刻意不返回可区分的状态码 (spec §3.3): 网页界面关着时是 404,
//! 开着时是 401。而 app 每次重启都会轮换密钥 —— 所以拿到 404 / 401 / 403 或连不上时,
//! 一律先重读 runtime.json 重试一次, 仍然 404 / 401 才判定为「终端界面未启用」。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::Mutex;

use super::discovery::{read_runtime, DiscoveryError, RuntimeInfo};
use super::sse::{SseEvent, SseParser};

pub const LOCAL_HEADER: &str = "x-ccr-local";
pub const CSRF_HEADER: &str = "x-ccr-ui";

/// **用户可见文字一律走 `i18n::client_error`, 不看这里的 `Display`**——这个 derive 现在是英文
/// 开发者文字, 只给日志 / `Debug` 场景用。
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    /// 连接被拒: runtime.json 在, 但那个端口上没有进程。
    #[error("cc-router is not running")]
    NotRunning,
    /// 重读密钥重试后仍是 404 / 401。
    #[error("terminal UI is disabled")]
    Disabled,
    #[error("{message} ({code}, HTTP {status})")]
    Api { status: u16, code: String, message: String },
    #[error("network error: {0}")]
    Transport(String),
    #[error("response could not be decoded: {0}")]
    Decode(String),
    /// 读本地 CA 证书失败 (只有走 https 才会碰到)。
    #[error("failed to read {path}: {message}")]
    ReadFile { path: String, message: String },
}

struct Conn {
    info: RuntimeInfo,
    http: reqwest::Client,
}

pub struct Client {
    data_dir: PathBuf,
    conn: Mutex<Conn>,
}

fn build_http(info: &RuntimeInfo) -> Result<reqwest::Client, ClientError> {
    let mut b = reqwest::Client::builder().connect_timeout(Duration::from_secs(3));
    // 只有走 https 时才需要信任本地 CA; 只加这一张, 不关证书校验。
    if info.http_port.is_none() {
        if let Some(path) = info.ca_pem_path.as_deref() {
            let pem = std::fs::read(path).map_err(|e| ClientError::ReadFile { path: path.to_string(), message: e.to_string() })?;
            let cert = reqwest::Certificate::from_pem(&pem).map_err(|e| ClientError::Transport(e.to_string()))?;
            b = b.add_root_certificate(cert);
        }
    }
    b.build().map_err(|e| ClientError::Transport(e.to_string()))
}

fn connect(data_dir: &Path) -> Result<Conn, ClientError> {
    conn_from(read_runtime(data_dir)?)
}

fn conn_from(info: RuntimeInfo) -> Result<Conn, ClientError> {
    let http = build_http(&info)?;
    Ok(Conn { info, http })
}

/// 两种错误体: 命令层 `{"code","message"}`; 中间件层 Anthropic 风格 `{"error":{"type","message"}}`。
fn api_error(status: u16, body: &str) -> ClientError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let pick = |a: &Value, k: &str| a.get(k).and_then(Value::as_str).map(str::to_string);
    let (code, message) = match v.get("error") {
        Some(e) => (pick(e, "type"), pick(e, "message")),
        None => (pick(&v, "code"), pick(&v, "message")),
    };
    ClientError::Api {
        status,
        code: code.unwrap_or_else(|| format!("http_{status}")),
        message: message.unwrap_or_else(|| body.chars().take(200).collect()),
    }
}

enum Attempt<T> {
    Done(T),
    /// 值得重读 runtime.json 再试一次的失败。
    Stale(ClientError),
}

impl Client {
    pub fn connect(data_dir: impl Into<PathBuf>) -> Result<Self, ClientError> {
        let data_dir = data_dir.into();
        let conn = connect(&data_dir)?;
        Ok(Self { data_dir, conn: Mutex::new(conn) })
    }

    /// 用调用方已经读好的 runtime.json 建连接 (入口先读一次拿连接前语言, 不必再读第二次)。
    /// 之后的重连照常按 `data_dir` 重读。
    pub fn from_runtime(data_dir: impl Into<PathBuf>, info: RuntimeInfo) -> Result<Self, ClientError> {
        let conn = conn_from(info)?;
        Ok(Self { data_dir: data_dir.into(), conn: Mutex::new(conn) })
    }

    pub async fn runtime(&self) -> RuntimeInfo {
        self.conn.lock().await.info.clone()
    }

    async fn reload(&self) -> Result<(), ClientError> {
        let fresh = connect(&self.data_dir)?;
        *self.conn.lock().await = fresh;
        Ok(())
    }

    async fn send(&self, req: impl Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder) -> Result<Attempt<reqwest::Response>, ClientError> {
        let (http, base, secret) = {
            let c = self.conn.lock().await;
            let base = c.info.base_url().ok_or(ClientError::NotRunning)?;
            (c.http.clone(), base, c.info.local_secret.clone())
        };
        let resp = req(&http, &base).header(LOCAL_HEADER, secret).header(CSRF_HEADER, "1").send().await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) if e.is_connect() => return Ok(Attempt::Stale(ClientError::NotRunning)),
            Err(e) => return Err(ClientError::Transport(e.to_string())),
        };
        let status = resp.status().as_u16();
        if status == 401 || status == 403 || status == 404 {
            let body = resp.text().await.unwrap_or_default();
            // 命令层的错误体形如 {"code": "...", ...} (字符串 code) —— 那是真的 API 错误
            // (订阅不存在 / 命令名拼错等), 重读密钥没用。网关的 404 (空体) 与会话层的 401
            // (Anthropic 风格, 无顶层 code) 都不满足这个形状, 仍走「密钥过期」重试路径。
            let has_command_layer_code =
                serde_json::from_str::<Value>(&body).ok().and_then(|v| v.get("code").and_then(Value::as_str).map(str::to_string)).is_some();
            if has_command_layer_code {
                return Err(api_error(status, &body));
            }
            let err = if status == 403 { api_error(status, &body) } else { ClientError::Disabled };
            return Ok(Attempt::Stale(err));
        }
        Ok(Attempt::Done(resp))
    }

    async fn send_with_retry(&self, req: impl Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder) -> Result<reqwest::Response, ClientError> {
        match self.send(&req).await? {
            Attempt::Done(r) => Ok(r),
            Attempt::Stale(first) => {
                // runtime.json 没了 / 坏了 → 报原始错误, 它更接近真相
                if self.reload().await.is_err() {
                    return Err(first);
                }
                match self.send(&req).await? {
                    Attempt::Done(r) => Ok(r),
                    Attempt::Stale(e) => Err(e),
                }
            }
        }
    }

    /// `args` 是 camelCase 的参数对象, 无参数传 `json!({})`。15 秒超时。
    pub async fn call<T: DeserializeOwned>(&self, name: &str, args: Value) -> Result<T, ClientError> {
        self.call_with_timeout(name, args, Duration::from_secs(15)).await
    }

    /// 与 `call` 相同, 但单次请求的超时由调用方给 (测试连接要真的打一次上游, 15 秒不够)。
    pub async fn call_with_timeout<T: DeserializeOwned>(&self, name: &str, args: Value, timeout: Duration) -> Result<T, ClientError> {
        let resp =
            self.send_with_retry(|http, base| http.post(format!("{base}/ui/api/cmd/{name}")).timeout(timeout).json(&args)).await?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| ClientError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(api_error(status.as_u16(), &body));
        }
        serde_json::from_str(&body).map_err(|e| ClientError::Decode(format!("{name}: {e}")))
    }

    pub async fn events(&self) -> Result<EventStream, ClientError> {
        let resp = self.send_with_retry(|http, base| http.get(format!("{base}/ui/api/events"))).await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            return Err(api_error(status, &resp.text().await.unwrap_or_default()));
        }
        Ok(EventStream { resp, parser: SseParser::default(), pending: VecDeque::new(), idle: None })
    }
}

pub struct EventStream {
    resp: reqwest::Response,
    parser: SseParser,
    pending: VecDeque<SseEvent>,
    idle: Option<Duration>,
}

impl EventStream {
    /// 两次收到**任何字节** (含 `: keepalive` 保活注释) 之间最多等 `idle`; 超时 `next()` 返回
    /// `Err(ClientError::Transport("idle timeout".into()))`。不调用 = 不限时 (原行为)。
    pub fn with_idle_timeout(mut self, idle: Duration) -> Self {
        self.idle = Some(idle);
        self
    }

    /// `Ok(None)` = 服务端关闭了连接 (app 退出)。调用方负责退避重连。
    pub async fn next(&mut self) -> Result<Option<SseEvent>, ClientError> {
        loop {
            if let Some(ev) = self.pending.pop_front() {
                return Ok(Some(ev));
            }
            let chunk = match self.idle {
                // 任何字节到达 (含保活注释) 都让 chunk() 返回, 也就重置了这次 timeout 的计时。
                Some(idle) => match tokio::time::timeout(idle, self.resp.chunk()).await {
                    Ok(r) => r,
                    Err(_) => return Err(ClientError::Transport("idle timeout".into())),
                },
                None => self.resp.chunk().await,
            };
            match chunk {
                Ok(Some(bytes)) => self.pending.extend(self.parser.push(&bytes)),
                Ok(None) => return Ok(None),
                Err(e) => return Err(ClientError::Transport(e.to_string())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::discovery::RUNTIME_FILE;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn write_runtime(dir: &Path, port: u16, secret: &str) {
        let body = json!({"pid":1,"app_version":"5.1.0","http_port":port,"https_port":null,"ca_pem_path":null,"local_secret":secret});
        std::fs::write(dir.join(RUNTIME_FILE), body.to_string()).unwrap();
    }

    fn port_of(server: &MockServer) -> u16 {
        server.address().port()
    }

    #[tokio::test]
    async fn call_sends_both_headers_and_decodes_the_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ui/api/cmd/proxy_status"))
            .and(header(LOCAL_HEADER, "s3cret"))
            .and(header(CSRF_HEADER, "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"running": true})))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), port_of(&server), "s3cret");

        let got: Value = Client::connect(dir.path()).unwrap().call("proxy_status", json!({})).await.unwrap();
        assert_eq!(got["running"], true);
    }

    /// app 重启 → 密钥轮换。TUI 手里的旧密钥换来 401 (网页界面开着时), 必须重读文件自愈,
    /// 而不是告诉用户「终端界面未启用」。
    #[tokio::test]
    async fn stale_secret_is_healed_by_rereading_the_runtime_file() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header(LOCAL_HEADER, "new"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!(42)))
            .mount(&server)
            .await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(401)).mount(&server).await;

        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), port_of(&server), "old");
        let client = Client::connect(dir.path()).unwrap();
        write_runtime(dir.path(), port_of(&server), "new"); // app "重启" 了

        let got: u32 = client.call("anything", json!({})).await.unwrap();
        assert_eq!(got, 42);
        assert_eq!(client.runtime().await.local_secret, "new");
    }

    #[tokio::test]
    async fn persistent_404_or_401_means_disabled() {
        for status in [404u16, 401] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(status)).expect(2).mount(&server).await;
            let dir = tempfile::tempdir().unwrap();
            write_runtime(dir.path(), port_of(&server), "s");
            let err = Client::connect(dir.path()).unwrap().call::<Value>("x", json!({})).await.unwrap_err();
            assert!(matches!(err, ClientError::Disabled), "{status}: {err}");
        }
    }

    #[tokio::test]
    async fn unknown_command_404_is_an_api_error_and_is_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"code":"unknown_command","message":"unknown command: nope"})))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), port_of(&server), "s");
        let err = Client::connect(dir.path()).unwrap().call::<Value>("nope", json!({})).await.unwrap_err();
        assert!(matches!(&err, ClientError::Api { status: 404, code, .. } if code == "unknown_command"), "{err}");
    }

    /// 命令层的 404 (订阅不存在等) 带 JSON 体, 是真的 API 错误, 不能当成密钥过期去重试。
    #[tokio::test]
    async fn command_layer_404_with_code_is_an_api_error_and_is_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"code":"subscription_not_found","message":"订阅不存在"})))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), port_of(&server), "s");
        let err = Client::connect(dir.path()).unwrap().call::<Value>("get_subscription", json!({"id":"x"})).await.unwrap_err();
        assert!(matches!(&err, ClientError::Api { status: 404, code, .. } if code == "subscription_not_found"), "{err}");
    }

    /// runtime.json 在两次尝试之间消失了 (app 退出后被清理): 报告的应是第一次的判断, 而不是「找不到文件」。
    #[tokio::test]
    async fn reload_failure_surfaces_the_original_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(401)).expect(1).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), port_of(&server), "s");
        let client = Client::connect(dir.path()).unwrap();
        std::fs::remove_file(dir.path().join(RUNTIME_FILE)).unwrap();
        let err = client.call::<Value>("x", json!({})).await.unwrap_err();
        assert!(matches!(err, ClientError::Disabled), "{err}");
    }

    #[tokio::test]
    async fn command_errors_carry_code_and_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({"code":"bad_request","message":"无效 id"})))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), port_of(&server), "s");
        let err = Client::connect(dir.path()).unwrap().call::<Value>("x", json!({})).await.unwrap_err();
        assert_eq!(err.to_string(), "无效 id (bad_request, HTTP 400)");
    }

    #[tokio::test]
    async fn nothing_listening_is_not_running() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        }; // listener dropped → 端口空闲
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), port, "s");
        let err = Client::connect(dir.path()).unwrap().call::<Value>("x", json!({})).await.unwrap_err();
        assert!(matches!(err, ClientError::NotRunning), "{err}");
    }

    /// 测试连接要真的打一次上游, 调用方可以给比默认 15 秒更短的超时。上游卡住时必须在给定
    /// 超时内返回 (不等 send_with_retry 的重试, 因为超时不是 `is_connect()` 错误)。
    #[tokio::test]
    async fn call_with_timeout_is_honoured() {
        let start = std::time::Instant::now();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(1500)))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), port_of(&server), "s");
        let client = Client::connect(dir.path()).unwrap();

        let err = client.call_with_timeout::<Value>("slow", json!({}), Duration::from_millis(200)).await.unwrap_err();
        assert!(matches!(err, ClientError::Transport(_)), "{err}");
        assert!(start.elapsed() < Duration::from_secs(1), "花了 {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn events_stream_yields_parsed_events_then_none_on_close() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ui/api/events"))
            .and(header(LOCAL_HEADER, "s"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                b": keepalive\n\nevent: events_flushed\ndata: null\n\n".to_vec(),
                "text/event-stream",
            ))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), port_of(&server), "s");
        let mut stream = Client::connect(dir.path()).unwrap().events().await.unwrap();
        assert_eq!(stream.next().await.unwrap(), Some(SseEvent { name: "events_flushed".into(), data: "null".into() }));
        assert_eq!(stream.next().await.unwrap(), None);
    }

    /// Task 3: 保活注释本身不含事件, 但每收到一次都要重置空闲计时——五段保活之间各隔 60ms
    /// (< 200ms 的 idle), 之后紧跟的真事件必须照常收到, 而不是被中途的假「空闲」判断打断。
    #[tokio::test]
    async fn keepalives_keep_an_idle_stream_alive() {
        let script = vec![
            (Duration::from_millis(60), b": keepalive\n\n".as_slice()),
            (Duration::from_millis(60), b": keepalive\n\n".as_slice()),
            (Duration::from_millis(60), b": keepalive\n\n".as_slice()),
            (Duration::from_millis(60), b": keepalive\n\n".as_slice()),
            (Duration::from_millis(60), b": keepalive\n\n".as_slice()),
            (Duration::from_millis(60), b"event: e\ndata: 1\n\n".as_slice()),
        ];
        let addr = crate::client::test_support::trickle_server(script).await;
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), addr.port(), "s");
        let mut stream = Client::connect(dir.path()).unwrap().events().await.unwrap().with_idle_timeout(Duration::from_millis(200));

        let ev = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("五段保活加起来不到 1 秒, next() 应该在这之内拿到事件")
            .unwrap();
        assert_eq!(ev, Some(SseEvent { name: "e".into(), data: "1".into() }));
    }

    /// Task 3: 连接建立后上游彻底沉默 (黑洞), `idle` 超时后 `next()` 必须主动放弃, 而不是永远
    /// 卡在 `chunk()` 里等一个再也不会来的字节。
    #[tokio::test]
    async fn a_silent_stream_times_out() {
        let script = vec![(Duration::from_millis(0), b": keepalive\n\n".as_slice())];
        let addr = crate::client::test_support::trickle_server(script).await;
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path(), addr.port(), "s");
        let mut stream = Client::connect(dir.path()).unwrap().events().await.unwrap().with_idle_timeout(Duration::from_millis(200));

        let err = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("idle (200ms) 应该远早于外层 1 秒的安全网就触发")
            .expect_err("黑洞之后应该超时, 不是正常返回");
        assert!(err.to_string().contains("idle"), "{err}");
    }
}
