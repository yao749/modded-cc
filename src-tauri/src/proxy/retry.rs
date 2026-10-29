//! 上游错误分类 + Retry-After 解析（设计稿 §5.2）。

use std::time::Duration;

use axum::http::HeaderMap;
use tracing::debug;

#[derive(Debug, Clone, Copy)]
pub enum ShouldRetry {
    Yes(RetryReason),
    No,
}

#[derive(Debug, Clone, Copy)]
pub enum RetryReason {
    AuthFailed,
    RateLimited,
    ServerError,
    Network,
    StreamAbortedBeforeAnyEvent,
}

/// Retry-After 解析的 clamp 边界。
/// 猜长比猜短安全：过短会触发反复限流，过长白跑配额。
pub const RETRY_AFTER_MIN_SECS: u64 = 30;
// 上限 30min，与 subscription::state_machine 中 QuotaExhausted 的硬编码冷却默认值(1800s)对齐，
// 避免"Retry-After 上界"和"无头默认冷却"两个上界语义打架。
pub const RETRY_AFTER_MAX_SECS: u64 = 1800; // 30min

/// 纯字符串版 Retry-After 解析。接受 trim 后的原始值，支持三种合法格式：
/// - 纯秒数 (HTTP-date delta-seconds): "60", "3600"
/// - RFC 2822 日期: "Thu, 01 Jan 2026 00:00:00 GMT"
/// - RFC 3339 日期: "2026-01-01T00:00:00Z"
///
/// 非法/损坏格式静默返回 None。解析成功后 clamp 到 [30s, 30min]。
/// 抽成纯函数是为了 upstream.rs (reqwest::HeaderMap) 和 dispatch 层 (axum::HeaderMap) 共用。
pub fn parse_retry_after_value(raw: &str) -> Option<Duration> {
    if raw.is_empty() {
        return None;
    }

    let secs = if let Ok(n) = raw.parse::<u64>() {
        // 纯秒数
        n
    } else if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(raw) {
        // RFC 2822: 计算与当前时间差
        let now = chrono::Utc::now();
        (dt.with_timezone(&chrono::Utc) - now).num_seconds().max(0) as u64
    } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        // RFC 3339
        let now = chrono::Utc::now();
        (dt.with_timezone(&chrono::Utc) - now).num_seconds().max(0) as u64
    } else {
        debug!(value = %raw, "Retry-After 头格式不认识, 静默跳过");
        return None;
    };

    let clamped = secs.clamp(RETRY_AFTER_MIN_SECS, RETRY_AFTER_MAX_SECS);
    Some(Duration::from_secs(clamped))
}

/// 从 axum 响应头里取 Retry-After 并解析（dispatch 层用）。
pub fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let raw = headers
        .get("retry-after")
        .or_else(|| headers.get("Retry-After"))
        .and_then(|v| v.to_str().ok())
        .map(str::trim)?;
    parse_retry_after_value(raw)
}

impl RetryReason {
    /// 触发事件时默认等待（目前没有实际退避，仅记录语义）。
    pub fn _suggested_backoff(self) -> Duration {
        match self {
            Self::AuthFailed => Duration::ZERO,
            Self::RateLimited => Duration::ZERO,
            Self::ServerError => Duration::ZERO,
            Self::Network => Duration::ZERO,
            Self::StreamAbortedBeforeAnyEvent => Duration::ZERO,
        }
    }
}

/// 按 HTTP 状态决定是否应该切订阅重试。
///
/// 调度策略 (2026-04 修订):
/// - 2xx → 不重试,返回客户端
/// - 401/403 → 切下家,触发 AuthFailed 状态
/// - 429 → 切下家,触发 RateLimited 状态 + 60s 冷却
/// - 5xx → 切下家,累积 3 次进 TransientError + 30s 起递增冷却
/// - **其他 4xx (含 400/402/404/422 等) → 切下家但不冷却**
///   原因: provider 之间能力差异 (如 thinking 块支持) 会让同一份请求在 A 上 200 在 B 上 400。
///   保留 "不冷却" 语义避免客户端 bug 把所有订阅冷却 (state_machine::classify_http 中处理)。
/// - 其他状态码 → 不重试 (兜底)
pub fn classify_response(status: u16, _body: Option<&str>) -> ShouldRetry {
    match status {
        200..=299 => ShouldRetry::No,
        401 | 403 => ShouldRetry::Yes(RetryReason::AuthFailed),
        429 => ShouldRetry::Yes(RetryReason::RateLimited),
        500..=599 => ShouldRetry::Yes(RetryReason::ServerError),
        400..=499 => ShouldRetry::Yes(RetryReason::ServerError),
        _ => ShouldRetry::No,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_yes(s: ShouldRetry) -> bool {
        matches!(s, ShouldRetry::Yes(_))
    }

    #[test]
    fn success_does_not_retry() {
        assert!(matches!(classify_response(200, None), ShouldRetry::No));
        assert!(matches!(classify_response(204, None), ShouldRetry::No));
    }

    #[test]
    fn auth_failed_retries_with_auth_reason() {
        assert!(matches!(
            classify_response(401, None),
            ShouldRetry::Yes(RetryReason::AuthFailed)
        ));
        assert!(matches!(
            classify_response(403, None),
            ShouldRetry::Yes(RetryReason::AuthFailed)
        ));
    }

    #[test]
    fn rate_limited_retries_with_rate_limit_reason() {
        assert!(matches!(
            classify_response(429, None),
            ShouldRetry::Yes(RetryReason::RateLimited)
        ));
    }

    #[test]
    fn server_error_retries() {
        assert!(matches!(
            classify_response(500, None),
            ShouldRetry::Yes(RetryReason::ServerError)
        ));
        assert!(matches!(
            classify_response(503, None),
            ShouldRetry::Yes(RetryReason::ServerError)
        ));
    }

    #[test]
    fn other_4xx_now_retries() {
        // 修复前: 400/402/404/422 全都 No, 任务暂停
        // 修复后: 都跳下家, state_machine 不冷却
        assert!(is_yes(classify_response(400, None)));
        assert!(is_yes(classify_response(402, None)));
        assert!(is_yes(classify_response(404, None)));
        assert!(is_yes(classify_response(422, None)));
    }

    #[test]
    fn parse_retry_after_delta_seconds() {
        let mut h = HeaderMap::new();
        h.insert("retry-after", "60".parse().unwrap());
        assert_eq!(parse_retry_after(&h), Some(Duration::from_secs(60)));

        // 900 在 [30, 1800] 界内, 原样返回
        h.insert("retry-after", "900".parse().unwrap());
        assert_eq!(parse_retry_after(&h), Some(Duration::from_secs(900)));
    }

    #[test]
    fn parse_retry_after_clamps_to_bounds() {
        // 过短: 上游说 3 秒, clamp 到 30 秒
        let mut h = HeaderMap::new();
        h.insert("retry-after", "3".parse().unwrap());
        assert_eq!(parse_retry_after(&h), Some(Duration::from_secs(30)));

        // 过长: 上游说 1 小时, clamp 到 30 分钟(与 QuotaExhausted 默认冷却对齐)
        let mut h = HeaderMap::new();
        h.insert("retry-after", "3600".parse().unwrap());
        assert_eq!(parse_retry_after(&h), Some(Duration::from_secs(1800)));
    }

    #[test]
    fn parse_retry_after_rfc2822_future() {
        let mut h = HeaderMap::new();
        let future = (chrono::Utc::now() + chrono::Duration::seconds(120))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        h.insert("retry-after", future.parse().unwrap());
        let got = parse_retry_after(&h).unwrap();
        assert!(got >= Duration::from_secs(30) && got <= Duration::from_secs(240));
    }

    #[test]
    fn parse_retry_after_rfc3339_future() {
        let mut h = HeaderMap::new();
        let future = (chrono::Utc::now() + chrono::Duration::seconds(120))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        h.insert("retry-after", future.parse().unwrap());
        let got = parse_retry_after(&h).unwrap();
        assert!(got >= Duration::from_secs(30) && got <= Duration::from_secs(240));
    }

    #[test]
    fn parse_retry_after_no_header() {
        let h = HeaderMap::new();
        assert_eq!(parse_retry_after(&h), None);
    }

    #[test]
    fn parse_retry_after_garbage_silent() {
        let mut h = HeaderMap::new();
        h.insert("retry-after", "not-a-date".parse().unwrap());
        assert_eq!(parse_retry_after(&h), None);

        let mut h = HeaderMap::new();
        h.insert("retry-after", "".parse().unwrap());
        assert_eq!(parse_retry_after(&h), None);
    }
}
