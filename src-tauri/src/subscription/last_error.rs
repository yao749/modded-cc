//! 订阅「最近错误」的结构化形式。
//!
//! `last_error_message` 是一段中文字符串, 落 DB (`subscriptions.last_error_message`) 也进事件 payload,
//! TUI 原样显示它。桌面端要按界面语言显示, 所以 DTO / 事件 payload 另带一份 [`LastError`],
//! 前端优先用它、缺失时退回原文。
//!
//! 文本与结构互为唯一来源: 状态机只经 [`LastError::message`] 写文本, DTO 构造时用
//! [`LastError::parse`] 从文本 (含老版本落库的) 还原; 两者的往返由单测锁住, 改文案必须两边一起改。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum LastError {
    /// 401 / 403。
    AuthFailed { status: u16 },
    /// 429。
    RateLimited,
    /// 5xx。
    ServerError { status: u16 },
    Network,
    /// 流内 SSE error, 判定为长期配额耗尽 (智谱 5h / 月度)。
    UpstreamQuotaExhausted,
    /// 流内 SSE error, 判定为短期限速。
    UpstreamRateLimited,
}

const NETWORK: &str = "network error";
const UPSTREAM_QUOTA: &str = "上游 SSE error: 配额耗尽 (5h/月度)";
const UPSTREAM_RATE: &str = "上游 SSE error: 速率限制";
const RATE_LIMITED: &str = "HTTP 429: 限流";
const AUTH_SUFFIX: &str = ": 凭证失效";

impl LastError {
    /// 写进 `last_error_message` 的文本 (DB / 事件 payload / TUI 看到的就是它)。
    pub fn message(self) -> String {
        match self {
            Self::AuthFailed { status } => format!("HTTP {status}{AUTH_SUFFIX}"),
            Self::RateLimited => RATE_LIMITED.to_string(),
            Self::ServerError { status } => format!("HTTP {status}"),
            Self::Network => NETWORK.to_string(),
            Self::UpstreamQuotaExhausted => UPSTREAM_QUOTA.to_string(),
            Self::UpstreamRateLimited => UPSTREAM_RATE.to_string(),
        }
    }

    /// [`message`](Self::message) 的逆运算; 认不出的文本返回 `None` (前端退回原文)。
    pub fn parse(msg: &str) -> Option<Self> {
        match msg {
            NETWORK => return Some(Self::Network),
            UPSTREAM_QUOTA => return Some(Self::UpstreamQuotaExhausted),
            UPSTREAM_RATE => return Some(Self::UpstreamRateLimited),
            RATE_LIMITED => return Some(Self::RateLimited),
            _ => {}
        }
        let rest = msg.strip_prefix("HTTP ")?;
        if let Some(code) = rest.strip_suffix(AUTH_SUFFIX) {
            let status = code.parse().ok()?;
            return matches!(status, 401 | 403).then_some(Self::AuthFailed { status });
        }
        let status: u16 = rest.parse().ok()?;
        (500..=599).contains(&status).then_some(Self::ServerError { status })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [LastError; 8] = [
        LastError::AuthFailed { status: 401 },
        LastError::AuthFailed { status: 403 },
        LastError::RateLimited,
        LastError::ServerError { status: 500 },
        LastError::ServerError { status: 503 },
        LastError::Network,
        LastError::UpstreamQuotaExhausted,
        LastError::UpstreamRateLimited,
    ];

    #[test]
    fn message_round_trips() {
        for e in ALL {
            assert_eq!(LastError::parse(&e.message()), Some(e), "{e:?}");
        }
    }

    #[test]
    fn unknown_text_is_none() {
        for s in ["", "HTTP", "HTTP 404", "HTTP 200: 凭证失效", "HTTP abc", "上游 429", "boom"] {
            assert_eq!(LastError::parse(s), None, "{s:?}");
        }
    }

    #[test]
    fn serializes_as_tagged_code() {
        let v = serde_json::to_value(LastError::AuthFailed { status: 401 }).unwrap();
        assert_eq!(v, serde_json::json!({ "code": "auth_failed", "status": 401 }));
        let v = serde_json::to_value(LastError::Network).unwrap();
        assert_eq!(v, serde_json::json!({ "code": "network" }));
    }
}
