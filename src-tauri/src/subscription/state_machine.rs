//! 订阅健康状态机（设计稿 §6）。
//!
//! 每次转换都是原子的：调用方持有 `SubscriptionRuntime` 的写锁，调用一个
//! `apply(...)`；副作用仅限于更新 DB 和发事件。
//!
//! 冷却定时器用 `tokio::spawn` + `sleep_until` 实现：到期后回调再次
//! 调用 `apply(CooldownExpired)`。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::AppResult;
use crate::observability::events::{self, EventEntry};
use crate::subscription::last_error::LastError;
use crate::subscription::model::{SubscriptionRuntime, SubscriptionState};
use crate::subscription::store;

/// Retry-After 冷却的 clamp 范围: 秒.
/// 上游给 1 秒 (突发 429) 太短, 给 86400 (24h) 太长, 都不适合直接信.
/// 猜长比猜短安全: 冷却到期才会自动复活, recheck_worker ping 成功会走 UserManualReset 提前复活.
pub const RETRY_AFTER_MIN_SECS: u64 = 30;
pub const RETRY_AFTER_MAX_SECS: u64 = 7200; // 2h

/// 默认冷却秒数:
/// - RateLimited (RPM/TPM 类): 上游没给 Retry-After 时默认 60s (RPM 真的一分钟就恢复)
/// - QuotaExhausted (日/月配额): 上游没给 Retry-After 时默认 2h (猜长比猜短安全, recheck ping 会补刀)
pub const DEFAULT_RATE_LIMITED_SECS: u64 = 60;
pub const DEFAULT_QUOTA_EXHAUSTED_SECS: u64 = 7200;

/// 外部事件驱动状态机。
#[derive(Debug, Clone)]
pub enum Event {
    RequestSucceeded,
    /// 上游 HTTP 状态码 + 可选 Retry-After header 解析结果 (None = 没 header / 解析失败 / 被 clamp 掉).
    HttpStatus { status: u16, retry_after_secs: Option<u64> },
    NetworkError,
    CooldownExpired,
    UserEnable,
    UserDisable,
    UserUpdateKey,
    /// 用户手动测试连接通过 → 任意状态(含 AuthFailed)直接复活到 Healthy。
    /// 与 `UserUpdateKey` 区别: 不要求 key 改动, 是通用复活入口。
    UserManualReset,
    /// 上游 HTTP 200 但 SSE 流首事件是 `event: error` (智谱 1302/1308 这类业务限流).
    /// `is_quota_exhausted=true` → QuotaExhausted (长冷却); false → RateLimited (短冷却).
    UpstreamSseError {
        is_quota_exhausted: bool,
        /// 可选 Retry-After hint (有些 SSE error 响应 body 里也带 reset 时间).
        retry_after_secs: Option<u64>,
    },
}

/// 状态转换结果。
#[derive(Debug, Clone, Copy)]
pub struct Transition {
    pub state_changed: bool,
    pub new_state: SubscriptionState,
    pub schedule_cooldown: Option<Duration>,
}

pub fn apply<'a>(
    pool: &'a SqlitePool,
    app: &'a AppHandle,
    event_tx: &'a mpsc::Sender<EventEntry>,
    rt: Arc<RwLock<SubscriptionRuntime>>,
    event: Event,
) -> Pin<Box<dyn Future<Output = AppResult<Transition>> + Send + 'a>> {
    Box::pin(async move {
        let prev_state;
        let (transition, id, error_message_update) = {
            let mut guard = rt.write().await;
            prev_state = guard.state;
            transition(&mut guard, &event)
        };

        if transition.state_changed {
            let is_auth_failed = transition.new_state == SubscriptionState::AuthFailed;
            store::update_auth_failed(
                pool,
                &id,
                is_auth_failed,
                error_message_update.as_deref(),
            )
            .await?;

            let _ = app.emit("subscription_state_changed", &id.to_string());

            let display_name = rt.read().await.row.display_name.clone();
            let from_str = serde_json::to_value(prev_state)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let to_str = serde_json::to_value(transition.new_state)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            events::record_state_change(
                event_tx,
                id,
                format!("{}: {} → {}", display_name, from_str, to_str),
                serde_json::json!({
                    "from": prev_state,
                    "to": transition.new_state,
                    "reason": describe_event(&event),
                    "last_error": error_message_update,
                    "last_error_code": error_message_update.as_deref().and_then(LastError::parse),
                }),
            );
        }

        if let Some(duration) = transition.schedule_cooldown {
            let pool = pool.clone();
            let app = app.clone();
            let event_tx_clone = event_tx.clone();
            let rt = rt.clone();
            tokio::spawn(async move {
                tokio::time::sleep(duration).await;
                if let Err(e) = apply(&pool, &app, &event_tx_clone, rt, Event::CooldownExpired).await {
                    warn!(?e, "cooldown apply 失败");
                }
            });
        }

        Ok(transition)
    })
}

fn describe_event(e: &Event) -> &'static str {
    match e {
        Event::RequestSucceeded => "request_succeeded",
        Event::HttpStatus { .. } => "http_status",
        Event::NetworkError => "network_error",
        Event::CooldownExpired => "cooldown_expired",
        Event::UserEnable => "user_enable",
        Event::UserDisable => "user_disable",
        Event::UserUpdateKey => "user_update_key",
        Event::UserManualReset => "user_manual_reset",
        Event::UpstreamSseError { .. } => "upstream_sse_error",
    }
}

/// 解析 HTTP `Retry-After` header 为秒数, 支持两种合法格式:
/// - 纯数字秒数: `"60"` → 60
/// - HTTP-date (RFC 2822): `"Wed, 21 Oct 2015 07:28:00 GMT"` → 到那个时间点的剩余秒数
///
/// 结果会被 clamp 到 [30s, 2h], 超出范围的静默回退到 None.
/// 声明了但格式不认识 → None (让调用方走默认值).
pub fn parse_retry_after(value: &str, now: DateTime<Utc>) -> Option<u64> {
    let trimmed = value.trim();
    // 形式 1: 纯数字秒数
    if let Ok(secs) = trimmed.parse::<u64>() {
        return clamp_retry_after(secs);
    }
    // 形式 2: HTTP-date (RFC 2822 / IMF-fixdate) — chrono 内置 parser 直接支持 "Wed, 22 Oct 2025 12:30:00 GMT"
    if let Ok(dt) = DateTime::parse_from_rfc2822(trimmed) {
        let utc_dt = dt.with_timezone(&Utc);
        let delta = utc_dt.timestamp() - now.timestamp();
        if delta <= 0 {
            return None;
        }
        return clamp_retry_after(delta as u64);
    }
    // 形式 3: RFC 3339 / ISO 8601 — 某些上游用
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        let utc_dt = dt.with_timezone(&Utc);
        let delta = utc_dt.timestamp() - now.timestamp();
        if delta <= 0 {
            return None;
        }
        return clamp_retry_after(delta as u64);
    }
    None // 格式不认识, 回退默认值
}

fn clamp_retry_after(secs: u64) -> Option<u64> {
    if secs < RETRY_AFTER_MIN_SECS || secs > RETRY_AFTER_MAX_SECS {
        None
    } else {
        Some(secs)
    }
}

/// 把 Retry-After hint (None 或 Some) 和默认值合成最终冷却秒数.
/// 上游给的值如果在 clamp 范围内就优先信 header, 否则走 type-specific 默认值.
fn resolve_cooldown_secs(retry_after_secs: Option<u64>, default_secs: u64) -> u64 {
    retry_after_secs.unwrap_or(default_secs)
}

fn transition(
    rt: &mut SubscriptionRuntime,
    event: &Event,
) -> (Transition, Uuid, Option<String>) {
    let now = Utc::now();
    let prev_state = rt.state;
    let id = rt.row.id;
    let mut last_error_update: Option<String> = None;
    // 本次事件携带的 Retry-After hint (如果有), 供后面 match new_state 时决定冷却秒数.
    let mut pending_retry_after: Option<u64> = None;

    let new_state = match (rt.state, event) {
        // healthy ==================================
        (SubscriptionState::Healthy, Event::RequestSucceeded) => {
            rt.consecutive_errors = 0;
            rt.transient_error_level = 0;
            rt.last_error_message = None;
            SubscriptionState::Healthy
        }
        (SubscriptionState::Healthy, Event::HttpStatus { status, retry_after_secs }) => {
            pending_retry_after = *retry_after_secs;
            classify_http(*status, rt, &mut last_error_update)
        }
        (SubscriptionState::Healthy, Event::NetworkError) => {
            bump_transient(rt, LastError::Network, &mut last_error_update)
        }
        (SubscriptionState::Healthy, Event::UpstreamSseError { is_quota_exhausted, retry_after_secs }) => {
            pending_retry_after = *retry_after_secs;
            let err = if *is_quota_exhausted {
                LastError::UpstreamQuotaExhausted
            } else {
                LastError::UpstreamRateLimited
            };
            last_error_update = Some(err.message());
            rt.last_error_message = last_error_update.clone();
            if *is_quota_exhausted {
                SubscriptionState::QuotaExhausted
            } else {
                SubscriptionState::RateLimited
            }
        }
        (SubscriptionState::Healthy, Event::UserDisable) => {
            rt.row.enabled = false;
            SubscriptionState::Disabled
        }

        // rate_limited / quota_exhausted ============
        (SubscriptionState::RateLimited | SubscriptionState::QuotaExhausted, Event::CooldownExpired) => {
            rt.consecutive_errors = 0;
            SubscriptionState::Healthy
        }
        (SubscriptionState::RateLimited | SubscriptionState::QuotaExhausted, Event::UserEnable) => {
            rt.cooldown_until = None;
            SubscriptionState::Healthy
        }

        // transient_error ==========================
        (SubscriptionState::TransientError, Event::CooldownExpired) => {
            rt.consecutive_errors = 0;
            SubscriptionState::Healthy
        }
        (SubscriptionState::TransientError, Event::RequestSucceeded) => {
            rt.consecutive_errors = 0;
            rt.transient_error_level = 0;
            SubscriptionState::Healthy
        }

        // auth_failed ==============================
        (SubscriptionState::AuthFailed, Event::UserUpdateKey) => {
            rt.row.is_auth_failed = false;
            last_error_update = None;
            rt.last_error_message = None;
            SubscriptionState::Healthy
        }

        // disabled =================================
        (SubscriptionState::Disabled, Event::UserEnable) => {
            rt.row.enabled = true;
            rt.consecutive_errors = 0;
            rt.transient_error_level = 0;
            rt.cooldown_until = None;
            rt.last_error_message = None;
            SubscriptionState::Healthy
        }

        // 用户禁用是所有状态的汇聚点
        (_, Event::UserDisable) => {
            rt.row.enabled = false;
            SubscriptionState::Disabled
        }

        // 用户手动复活: 任意状态都直接清回 Healthy
        // (Disabled 不复活: 如果用户关掉了订阅, 测试连接也不应自动开起来)
        (state, Event::UserManualReset) if state != SubscriptionState::Disabled => {
            rt.row.is_auth_failed = false;
            rt.row.last_error_message = None;
            rt.last_error_message = None;
            rt.consecutive_errors = 0;
            rt.transient_error_level = 0;
            rt.cooldown_until = None;
            last_error_update = None;
            SubscriptionState::Healthy
        }

        // 其余情况保持状态
        _ => rt.state,
    };

    let mut schedule_cooldown = None;
    if new_state != prev_state {
        rt.state = new_state;
        rt.state_entered_at = now;
        rt.row.updated_at = now;
        match new_state {
            SubscriptionState::RateLimited => {
                // 优先信上游 Retry-After header (clamp 后), 没给/解析失败/超出范围 → 默认 60s (RPM 一分钟就恢复).
                let secs = resolve_cooldown_secs(pending_retry_after, DEFAULT_RATE_LIMITED_SECS);
                rt.cooldown_until = Some(now + chrono::Duration::seconds(secs as i64));
                schedule_cooldown = Some(Duration::from_secs(secs));
            }
            SubscriptionState::QuotaExhausted => {
                // 优先信上游 Retry-After header, 没给 → 默认 2h (猜长比猜短安全, recheck ping 会提前复活).
                let secs = resolve_cooldown_secs(pending_retry_after, DEFAULT_QUOTA_EXHAUSTED_SECS);
                rt.cooldown_until = Some(now + chrono::Duration::seconds(secs as i64));
                schedule_cooldown = Some(Duration::from_secs(secs));
            }
            SubscriptionState::TransientError => {
                let secs = match rt.transient_error_level {
                    0 => 30,
                    1 => 60,
                    2 => 120,
                    _ => 300,
                };
                rt.cooldown_until = Some(now + chrono::Duration::seconds(secs));
                schedule_cooldown = Some(Duration::from_secs(secs as u64));
                rt.transient_error_level = rt.transient_error_level.saturating_add(1);
            }
            SubscriptionState::AuthFailed => {
                rt.row.is_auth_failed = true;
                rt.cooldown_until = None;
            }
            SubscriptionState::Healthy => {
                rt.cooldown_until = None;
            }
            SubscriptionState::Disabled => {
                rt.cooldown_until = None;
            }
        }

        info!(
            id = %rt.row.id,
            ?prev_state,
            ?new_state,
            "subscription state transition"
        );
    }

    (
        Transition {
            state_changed: new_state != prev_state,
            new_state,
            schedule_cooldown,
        },
        id,
        last_error_update,
    )
}

fn classify_http(
    status: u16,
    rt: &mut SubscriptionRuntime,
    last_error: &mut Option<String>,
) -> SubscriptionState {
    match status {
        200..=299 => {
            rt.consecutive_errors = 0;
            rt.transient_error_level = 0;
            SubscriptionState::Healthy
        }
        400 => {
            // 请求内容问题，不计入错误（§5.2）
            SubscriptionState::Healthy
        }
        401 | 403 => {
            *last_error = Some(LastError::AuthFailed { status }.message());
            rt.last_error_message = last_error.clone();
            SubscriptionState::AuthFailed
        }
        429 => {
            *last_error = Some(LastError::RateLimited.message());
            rt.last_error_message = last_error.clone();
            // MVP: rate_limited 与 quota_exhausted 合并（§6.1）
            SubscriptionState::RateLimited
        }
        500..=599 => bump_transient(rt, LastError::ServerError { status }, last_error),
        _ => SubscriptionState::Healthy,
    }
}

fn bump_transient(
    rt: &mut SubscriptionRuntime,
    reason: LastError,
    last_error: &mut Option<String>,
) -> SubscriptionState {
    rt.consecutive_errors = rt.consecutive_errors.saturating_add(1);
    *last_error = Some(reason.message());
    rt.last_error_message = last_error.clone();
    if rt.consecutive_errors >= 3 {
        SubscriptionState::TransientError
    } else {
        SubscriptionState::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::model::SubscriptionRow;

    fn runtime() -> SubscriptionRuntime {
        let row = SubscriptionRow::test_fixture("anthropic", "api_paygo");
        SubscriptionRuntime::from_row(row)
    }

    #[test]
    fn healthy_to_rate_limited_on_429() {
        let mut rt = runtime();
        let (t, _, _) = transition(&mut rt, &Event::HttpStatus { status: 429, retry_after_secs: None });
        assert!(t.state_changed);
        assert_eq!(rt.state, SubscriptionState::RateLimited);
        assert!(rt.cooldown_until.is_some());
    }

    #[test]
    fn healthy_to_auth_failed_on_401() {
        let mut rt = runtime();
        let (t, _, _) = transition(&mut rt, &Event::HttpStatus { status: 401, retry_after_secs: None });
        assert!(t.state_changed);
        assert_eq!(rt.state, SubscriptionState::AuthFailed);
    }

    #[test]
    fn http_400_does_not_cool_down() {
        // 即便 retry.rs 把 400 改为可重试,state_machine 应保留「不冷却」语义
        // 避免客户端 bug 把所有订阅冷却。
        let mut rt = runtime();
        let (t, _, _) = transition(&mut rt, &Event::HttpStatus { status: 400, retry_after_secs: None });
        assert!(!t.state_changed);
        assert_eq!(rt.state, SubscriptionState::Healthy);
        assert!(rt.cooldown_until.is_none());
        assert_eq!(rt.consecutive_errors, 0);
    }

    #[test]
    fn other_4xx_does_not_cool_down() {
        // 404/422 等同 400 的语义: 跳下家但不冷却
        let mut rt = runtime();
        for status in [404, 422, 410, 408] {
            let (t, _, _) = transition(&mut rt, &Event::HttpStatus { status, retry_after_secs: None });
            assert!(!t.state_changed, "status {} should not change state", status);
            assert_eq!(rt.state, SubscriptionState::Healthy);
            assert!(rt.cooldown_until.is_none());
        }
        assert_eq!(rt.consecutive_errors, 0);
    }

    #[test]
    fn transient_error_after_three_fives() {
        let mut rt = runtime();
        for _ in 0..2 {
            let (t, _, _) = transition(&mut rt, &Event::HttpStatus { status: 500, retry_after_secs: None });
            assert!(!t.state_changed);
            assert_eq!(rt.state, SubscriptionState::Healthy);
        }
        let (t, _, _) = transition(&mut rt, &Event::HttpStatus { status: 500, retry_after_secs: None });
        assert!(t.state_changed);
        assert_eq!(rt.state, SubscriptionState::TransientError);
    }

    #[test]
    fn exponential_backoff_levels() {
        let mut rt = runtime();
        for _ in 0..3 {
            transition(&mut rt, &Event::HttpStatus { status: 500, retry_after_secs: None });
        }
        assert_eq!(rt.state, SubscriptionState::TransientError);
        let cooldown = rt.cooldown_until.unwrap();
        let delta = (cooldown - Utc::now()).num_seconds();
        assert!((28..=32).contains(&delta), "first cooldown ~30s, got {delta}");
    }

    #[test]
    fn cooldown_expired_returns_to_healthy() {
        let mut rt = runtime();
        for _ in 0..3 {
            transition(&mut rt, &Event::HttpStatus { status: 500, retry_after_secs: None });
        }
        let (t, _, _) = transition(&mut rt, &Event::CooldownExpired);
        assert!(t.state_changed);
        assert_eq!(rt.state, SubscriptionState::Healthy);
    }

    #[test]
    fn disabled_preserved_through_events() {
        let mut rt = runtime();
        transition(&mut rt, &Event::UserDisable);
        assert_eq!(rt.state, SubscriptionState::Disabled);
        let (t, _, _) = transition(&mut rt, &Event::RequestSucceeded);
        assert!(!t.state_changed);
        assert_eq!(rt.state, SubscriptionState::Disabled);
    }

    #[test]
    fn request_success_resets_error_count() {
        let mut rt = runtime();
        transition(&mut rt, &Event::HttpStatus { status: 500, retry_after_secs: None });
        transition(&mut rt, &Event::RequestSucceeded);
        assert_eq!(rt.consecutive_errors, 0);
    }

    #[test]
    fn auth_failed_only_clears_on_user_key_update() {
        let mut rt = runtime();
        transition(&mut rt, &Event::HttpStatus { status: 401, retry_after_secs: None });
        assert_eq!(rt.state, SubscriptionState::AuthFailed);
        let (t, _, _) = transition(&mut rt, &Event::CooldownExpired);
        assert!(!t.state_changed);
        let (t, _, _) = transition(&mut rt, &Event::UserUpdateKey);
        assert!(t.state_changed);
        assert_eq!(rt.state, SubscriptionState::Healthy);
    }

    #[test]
    fn manual_reset_revives_from_auth_failed() {
        let mut rt = runtime();
        transition(&mut rt, &Event::HttpStatus { status: 401, retry_after_secs: None });
        assert_eq!(rt.state, SubscriptionState::AuthFailed);
        assert!(rt.row.is_auth_failed);

        let (t, _, _) = transition(&mut rt, &Event::UserManualReset);
        assert!(t.state_changed);
        assert_eq!(rt.state, SubscriptionState::Healthy);
        assert!(!rt.row.is_auth_failed);
        assert!(rt.last_error_message.is_none());
    }

    #[test]
    fn manual_reset_clears_rate_limited_cooldown() {
        let mut rt = runtime();
        transition(&mut rt, &Event::HttpStatus { status: 429, retry_after_secs: None });
        assert_eq!(rt.state, SubscriptionState::RateLimited);
        assert!(rt.cooldown_until.is_some());

        let (t, _, _) = transition(&mut rt, &Event::UserManualReset);
        assert!(t.state_changed);
        assert_eq!(rt.state, SubscriptionState::Healthy);
        assert!(rt.cooldown_until.is_none());
    }

    #[test]
    fn manual_reset_clears_transient_error_level() {
        let mut rt = runtime();
        for _ in 0..3 {
            transition(&mut rt, &Event::HttpStatus { status: 500, retry_after_secs: None });
        }
        assert_eq!(rt.state, SubscriptionState::TransientError);
        assert!(rt.transient_error_level > 0);

        let (t, _, _) = transition(&mut rt, &Event::UserManualReset);
        assert!(t.state_changed);
        assert_eq!(rt.state, SubscriptionState::Healthy);
        assert_eq!(rt.transient_error_level, 0);
        assert_eq!(rt.consecutive_errors, 0);
    }

    #[test]
    fn manual_reset_does_not_unbreak_disabled() {
        let mut rt = runtime();
        transition(&mut rt, &Event::UserDisable);
        assert_eq!(rt.state, SubscriptionState::Disabled);

        // 手动重置不应自动启用被用户主动关掉的订阅
        let (t, _, _) = transition(&mut rt, &Event::UserManualReset);
        assert!(!t.state_changed);
        assert_eq!(rt.state, SubscriptionState::Disabled);
        assert!(!rt.row.enabled);
    }

    #[test]
    fn manual_reset_idempotent_on_healthy() {
        let mut rt = runtime();
        let (t, _, _) = transition(&mut rt, &Event::UserManualReset);
        assert!(!t.state_changed);
        assert_eq!(rt.state, SubscriptionState::Healthy);
    }

    #[test]
    fn healthy_to_rate_limited_on_sse_error_short() {
        let mut rt = runtime();
        let (t, _, _) = transition(
            &mut rt,
            &Event::UpstreamSseError { is_quota_exhausted: false, retry_after_secs: None },
        );
        assert!(t.state_changed);
        assert_eq!(rt.state, SubscriptionState::RateLimited);
        let cooldown_secs = (rt.cooldown_until.unwrap() - Utc::now()).num_seconds();
        assert!((58..=62).contains(&cooldown_secs), "rate_limited 60s, got {cooldown_secs}");
    }

    #[test]
    fn healthy_to_quota_exhausted_on_sse_error_long() {
        let mut rt = runtime();
        let (t, _, _) = transition(
            &mut rt,
            &Event::UpstreamSseError { is_quota_exhausted: true, retry_after_secs: None },
        );
        assert!(t.state_changed);
        assert_eq!(rt.state, SubscriptionState::QuotaExhausted);
        let cooldown_secs = (rt.cooldown_until.unwrap() - Utc::now()).num_seconds();
        assert!((7198..=7202).contains(&cooldown_secs), "quota_exhausted default 2h, got {cooldown_secs}");
    }

    #[test]
    fn rate_limited_returns_to_healthy_on_cooldown_expired() {
        // 与 quota_exhausted 共用 CooldownExpired 分支, 确保新状态也能复活
        let mut rt = runtime();
        transition(
            &mut rt,
            &Event::UpstreamSseError { is_quota_exhausted: true, retry_after_secs: None },
        );
        assert_eq!(rt.state, SubscriptionState::QuotaExhausted);
        let (t, _, _) = transition(&mut rt, &Event::CooldownExpired);
        assert!(t.state_changed);
        assert_eq!(rt.state, SubscriptionState::Healthy);
    }

    // ---------- retry_after header 优先测试 ----------

    #[test]
    fn rate_limited_uses_retry_after_header_not_default() {
        // 上游给了 Retry-After: 300 (5min), 应优先用 300 而非默认 60s
        let mut rt = runtime();
        transition(
            &mut rt,
            &Event::HttpStatus { status: 429, retry_after_secs: Some(300) },
        );
        assert_eq!(rt.state, SubscriptionState::RateLimited);
        let cooldown_secs = (rt.cooldown_until.unwrap() - Utc::now()).num_seconds();
        assert!((298..=302).contains(&cooldown_secs), "should use header 300, got {cooldown_secs}");
    }

    #[test]
    fn quota_exhausted_uses_retry_after_header_not_default() {
        // 上游给了 Retry-After: 1800 (30min), 应优先用 1800 而非默认 2h
        let mut rt = runtime();
        transition(
            &mut rt,
            &Event::HttpStatus { status: 429, retry_after_secs: Some(1800) },
        );
        assert_eq!(rt.state, SubscriptionState::RateLimited); // 429 是 RateLimited, QuotaExhausted 需 SSE error
        // 换一个: 429 但 retry_after 带 QuotaExhausted 状态需要 UpstreamSseError
        let mut rt2 = runtime();
        transition(
            &mut rt2,
            &Event::UpstreamSseError { is_quota_exhausted: true, retry_after_secs: Some(1800) },
        );
        assert_eq!(rt2.state, SubscriptionState::QuotaExhausted);
        let cooldown_secs = (rt2.cooldown_until.unwrap() - Utc::now()).num_seconds();
        assert!((1798..=1802).contains(&cooldown_secs), "should use header 1800, got {cooldown_secs}");
    }
}

// ---------- parse_retry_after 函数级测试 (不在 mod tests 里, 直接 pub fn) ----------

#[cfg(test)]
mod parse_retry_after_tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parse_seconds_form() {
        let now = Utc.with_ymd_and_hms(2025, 10, 22, 12, 0, 0).unwrap();
        assert_eq!(parse_retry_after("60", now), Some(60));
        assert_eq!(parse_retry_after("  120  ", now), Some(120)); // 空白容忍
    }

    #[test]
    fn parse_http_date_form() {
        let now = Utc.with_ymd_and_hms(2025, 10, 22, 12, 0, 0).unwrap();
        // IMF-fixdate: Wed, 22 Oct 2025 12:30:00 GMT → 30min = 1800s
        assert_eq!(parse_retry_after("Wed, 22 Oct 2025 12:30:00 GMT", now), Some(1800));
    }

    #[test]
    fn parse_iso8601_form() {
        let now = Utc.with_ymd_and_hms(2025, 10, 22, 12, 0, 0).unwrap();
        // RFC3339/ISO8601 形式 — 某些上游用这个
        assert_eq!(parse_retry_after("2025-10-22T12:15:00Z", now), Some(900)); // 15min = 900s
    }

    #[test]
    fn clamps_too_short() {
        // 1 秒太短 → 低于 MIN 30s → None, 调用方走默认值
        let now = Utc.with_ymd_and_hms(2025, 10, 22, 12, 0, 0).unwrap();
        assert_eq!(parse_retry_after("1", now), None);
        assert_eq!(parse_retry_after("10", now), None);
        assert_eq!(parse_retry_after("29", now), None);
        assert_eq!(parse_retry_after("30", now), Some(30)); // 边界值 OK
    }

    #[test]
    fn clamps_too_long() {
        // 24h 太长 → 超过 MAX 2h → None
        let now = Utc.with_ymd_and_hms(2025, 10, 22, 12, 0, 0).unwrap();
        assert_eq!(parse_retry_after("86400", now), None);
        assert_eq!(parse_retry_after("7201", now), None);
        assert_eq!(parse_retry_after("7200", now), Some(7200)); // 边界值 OK
    }

    #[test]
    fn garbage_falls_back_to_none() {
        // 格式不认 → None, 调用方走默认值
        let now = Utc.with_ymd_and_hms(2025, 10, 22, 12, 0, 0).unwrap();
        assert_eq!(parse_retry_after("not a number", now), None);
        assert_eq!(parse_retry_after("tomorrow", now), None);
        assert_eq!(parse_retry_after("", now), None);
        assert_eq!(parse_retry_after("  ", now), None);
    }

    #[test]
    fn expired_date_returns_none() {
        // HTTP-date 是过去时间 → 无意义, 回退默认值
        let now = Utc.with_ymd_and_hms(2025, 10, 22, 12, 0, 0).unwrap();
        assert_eq!(parse_retry_after("Tue, 21 Oct 2025 12:00:00 GMT", now), None);
    }
}
