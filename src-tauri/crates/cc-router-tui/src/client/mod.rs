pub mod discovery;
pub mod dto;
pub mod http;
pub mod sse;

pub use http::{Client, ClientError, EventStream};

/// TUI 调用的后端 command 名。集中在这里是为了让主 crate 的契约测试
/// (`src/tui_contract.rs::commands_are_registered`) 能逐个核对它们仍在 `web_commands!` 表里。
pub mod commands {
    pub const PROXY_STATUS: &str = "proxy_status";
    pub const GET_SETTINGS: &str = "get_settings";
    pub const LIST_SUBSCRIPTIONS: &str = "list_subscriptions";
    pub const GET_OVERALL_STATS: &str = "get_overall_stats";
    pub const GET_DAILY_SERIES: &str = "get_daily_series";
    pub const SET_SUBSCRIPTION_ENABLED: &str = "set_subscription_enabled";
    pub const TEST_CONNECTION: &str = "test_connection";
    pub const REFRESH_MODEL_LIST: &str = "refresh_model_list";
    pub const REFRESH_SUBSCRIPTION_BALANCE: &str = "refresh_subscription_balance";
    pub const UPDATE_SUBSCRIPTION: &str = "update_subscription";
    pub const LIST_VIRTUAL_MODELS: &str = "list_virtual_models";
    pub const UPDATE_VIRTUAL_MODEL: &str = "update_virtual_model";
    pub const LIST_REQUESTS: &str = "list_requests";
    /// 新建订阅向导: 拉厂商列表 / 创建 / 自定义厂商探测模型。
    pub const LIST_PROVIDERS: &str = "list_providers";
    pub const CREATE_SUBSCRIPTION: &str = "create_subscription";
    pub const PROBE_CUSTOM_MODELS: &str = "probe_custom_models";
    /// 订阅页 `d` 删除订阅。
    pub const DELETE_SUBSCRIPTION: &str = "delete_subscription";
    /// 契约测试遍历这张表; 加新 command 时同时加进来。
    pub const ALL: &[&str] = &[
        PROXY_STATUS,
        GET_SETTINGS,
        LIST_SUBSCRIPTIONS,
        GET_OVERALL_STATS,
        GET_DAILY_SERIES,
        SET_SUBSCRIPTION_ENABLED,
        TEST_CONNECTION,
        REFRESH_MODEL_LIST,
        REFRESH_SUBSCRIPTION_BALANCE,
        UPDATE_SUBSCRIPTION,
        LIST_VIRTUAL_MODELS,
        UPDATE_VIRTUAL_MODEL,
        LIST_REQUESTS,
        LIST_PROVIDERS,
        CREATE_SUBSCRIPTION,
        PROBE_CUSTOM_MODELS,
        DELETE_SUBSCRIPTION,
    ];
}

/// TUI 关心的后端事件名。集中在这里是为了让主 crate 的契约测试
/// (`src/tui_contract.rs::tui_event_names_are_bridged`) 能逐个核对它们仍在 `BRIDGED_EVENTS` 里。
pub mod events {
    pub const SUBSCRIPTION_STATE_CHANGED: &str = "subscription_state_changed";
    pub const SUBSCRIPTION_QUOTA_REACHED: &str = "subscription_quota_reached";
    pub const ROUTE_ATTEMPT_STARTED: &str = "route_attempt_started";
    pub const ROUTE_ATTEMPT_FINISHED: &str = "route_attempt_finished";
    /// 订阅列表可能变了、该重拉的两个事件 (总览 / 订阅 / 虚拟模型三页共用, 取代各自的 `SSE_REFETCH`)。
    pub const SUBSCRIPTION_CHANGES: &[&str] = &[SUBSCRIPTION_STATE_CHANGED, SUBSCRIPTION_QUOTA_REACHED];
    /// TUI 关心的全部事件名; `tui_contract.rs` 断言每一个都在后端 `BRIDGED_EVENTS` 里。
    pub const ALL: &[&str] = &[SUBSCRIPTION_STATE_CHANGED, SUBSCRIPTION_QUOTA_REACHED, ROUTE_ATTEMPT_STARTED, ROUTE_ATTEMPT_FINISHED];
}

/// 只给测试用的假后端 (比如测事件流空闲超时)。放在 `client` 而不是 `http` 里, 是因为
/// `http.rs` 与 `runtime.rs` 的测试都要用它。
#[cfg(test)]
pub(crate) mod test_support {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// 一次性的慢速 SSE 服务器: 只接受一个连接, 读完请求头 (直到看见 `\r\n\r\n`) 后回
    /// `HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n`,
    /// 然后按 `script` 逐段「等 delay → 写 bytes」, 写完后握住连接 10 秒不再发任何字节
    /// (模拟黑洞)。返回监听地址 (在 spawn 接收任务之前就已确定, 调用方不用等)。
    pub async fn trickle_server(script: Vec<(Duration, &'static [u8])>) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("绑定本地端口失败");
        let addr = listener.local_addr().expect("拿不到监听地址");
        tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            // 逐字节读到请求头结束 (\r\n\r\n) 为止——测试请求很小, 简单换取正确性足够。
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                if socket.read_exact(&mut byte).await.is_err() {
                    return;
                }
                buf.push(byte[0]);
                if buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let head = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
            if socket.write_all(head).await.is_err() {
                return;
            }
            for (delay, bytes) in script {
                tokio::time::sleep(delay).await;
                if socket.write_all(bytes).await.is_err() {
                    return;
                }
            }
            // 握住连接不再发任何字节, 模拟黑洞——调用方的 idle 超时应该在这之前就放弃。
            tokio::time::sleep(Duration::from_secs(10)).await;
        });
        addr
    }
}
