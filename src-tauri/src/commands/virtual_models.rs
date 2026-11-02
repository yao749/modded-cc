use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::virtual_model::model::{RoutingMode, RoutingParadigm, VirtualModelName};
use crate::virtual_model::store;

#[derive(Debug, Serialize)]
pub struct VirtualModelDto {
    pub name: String,
    pub mode: RoutingMode,
    pub subscription_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateVirtualModelInput {
    pub mode: RoutingMode,
    pub subscription_ids: Vec<String>,
}

#[tauri::command]
pub async fn list_virtual_models(state: State<'_, AppState>) -> AppResult<Vec<VirtualModelDto>> {
    let guard = state.virtual_models.read().await;
    let mut out = Vec::with_capacity(VirtualModelName::all().len());
    for name in VirtualModelName::all() {
        if let Some(cfg) = guard.get(&name) {
            out.push(VirtualModelDto {
                name: cfg.name.as_str().to_string(),
                mode: cfg.mode,
                subscription_ids: cfg.subscription_ids.iter().map(|id| id.to_string()).collect(),
            });
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn update_virtual_model(
    state: State<'_, AppState>,
    name: String,
    input: UpdateVirtualModelInput,
) -> AppResult<()> {
    let vm_name = VirtualModelName::parse(&name)
        .ok_or_else(|| AppError::UnknownVirtualModel(name.clone()))?;

    let ids: Result<Vec<Uuid>, _> = input
        .subscription_ids
        .iter()
        .map(|s| Uuid::parse_str(s))
        .collect();
    let ids = ids.map_err(|e| AppError::BadRequest(format!("invalid uuid: {e}")))?;

    // 校验: 所有 subscription 必须存在, 且端点协议与虚拟模型匹配 (model-jev ↔ systemone)。
    // 桌面 / 网页 / TUI 共用本命令, 前端过滤只是体验, 这里才是隔离边界。
    {
        let subs = state.subscriptions.read().await;
        for id in &ids {
            let rt = subs
                .get(id)
                .ok_or_else(|| AppError::SubscriptionNotFound(id.to_string()))?;
            let g = rt.read().await;
            if let Some(reason) = vm_name.binding_rejection(g.row.endpoint_protocol, &g.row.display_name) {
                return Err(AppError::BadRequest(reason));
            }
        }
    }

    // 去重保序
    let mut seen = std::collections::HashSet::new();
    let deduped: Vec<Uuid> = ids
        .into_iter()
        .filter(|id| seen.insert(*id))
        .collect();

    store::save_mode(&state.db, vm_name, input.mode).await?;
    store::save_bindings(&state.db, vm_name, &deduped).await?;

    let mut guard = state.virtual_models.write().await;
    if let Some(cfg) = guard.get_mut(&vm_name) {
        cfg.mode = input.mode;
        cfg.subscription_ids = deduped;
        cfg.last_used_index = 0;
    }
    Ok(())
}

/// 一次返回全部订阅的能力分类池条目 (按订阅 + 能力 + 优先级排序).
/// 池总览矩阵页需要: 逐个订阅 invoke `load_model_pool` 是 N 次往返。
#[tauri::command]
pub async fn list_model_pools(
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::subscription::store::ModelPoolEntry>> {
    crate::subscription::store::load_all_model_pools(&state.db).await
}

/// 池订阅的全局展示顺序 (供池页「订阅顺序」UI)。
///
/// 全局顺序没有单独的存储 —— 它是「各能力模型 `subscription_ids` 的合并顺序」。
/// 读取时按 [`POOL_ORDER_READ_ORDER`] 依次取各能力模型的绑定顺序, 首次出现即入列。
/// 执行过一次 [`reorder_pool_subscriptions`] 后, 5 个能力模型共享同一相对顺序,
/// 这里就会原样读回用户设定的顺序。
#[tauri::command]
pub async fn list_pool_subscription_order(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    let guard = state.virtual_models.read().await;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for name in POOL_ORDER_READ_ORDER {
        if let Some(cfg) = guard.get(&name) {
            for id in &cfg.subscription_ids {
                if seen.insert(*id) {
                    out.push(id.to_string());
                }
            }
        }
    }
    Ok(out)
}

/// 读全局顺序时优先参考的能力模型: 主对话最常用, 放第一位最贴近直觉。
const POOL_ORDER_READ_ORDER: [VirtualModelName; 5] = [
    VirtualModelName::Llm,
    VirtualModelName::Vision,
    VirtualModelName::Multimodal,
    VirtualModelName::Speech,
    VirtualModelName::Embedding,
];

/// 按给定的全局顺序重排所有能力池模型的订阅绑定 (只改顺序, 不改成员集)。
///
/// 池页「订阅顺序」上下箭头每次换位都调用它: 一次把 5 个能力模型的
/// `subscription_ids` 同时排成用户给定的相对顺序。membership 保持不变 ——
/// order 里没有的订阅 (理论上不该出现) 保持原相对顺序并落在末尾。
#[tauri::command]
pub async fn reorder_pool_subscriptions(
    state: State<'_, AppState>,
    order: Vec<String>,
) -> AppResult<()> {
    let ids: Result<Vec<Uuid>, _> = order.iter().map(|s| Uuid::parse_str(s)).collect();
    let ids = ids.map_err(|e| AppError::BadRequest(format!("invalid uuid: {e}")))?;
    let rank: std::collections::HashMap<Uuid, usize> =
        ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();

    // 1. 读快照, 算出每个能力模型「重排后的顺序」; 仅重排已绑定成员, 不增不删。
    let changes: Vec<(VirtualModelName, Vec<Uuid>)> = {
        let guard = state.virtual_models.read().await;
        let mut out = Vec::new();
        for name in VirtualModelName::all() {
            if name.paradigm() != RoutingParadigm::Pool {
                continue;
            }
            let Some(cfg) = guard.get(&name) else { continue };
            if cfg.subscription_ids.is_empty() {
                continue;
            }
            let before = cfg.subscription_ids.clone();
            let after = reorder_by_rank(&before, &rank);
            if after != before {
                out.push((name, after));
            }
        }
        out
    };

    if changes.is_empty() {
        return Ok(());
    }

    // 2. 落库 (不持锁, 与 update_virtual_model / sync_capability_bindings_from_pool 同序)
    for (name, new_ids) in &changes {
        store::save_bindings(&state.db, *name, new_ids).await?;
    }

    // 3. 更新内存表
    let mut guard = state.virtual_models.write().await;
    for (name, new_ids) in changes {
        if let Some(cfg) = guard.get_mut(&name) {
            cfg.subscription_ids = new_ids;
            cfg.last_used_index = 0;
        }
    }
    Ok(())
}

/// 只按 rank 稳定重排: rank 里有的按序号升序, 没有的保持原相对顺序落到末尾。
/// 不改成员集 (只重排 `before` 里已有的元素)。
fn reorder_by_rank(
    before: &[Uuid],
    rank: &std::collections::HashMap<Uuid, usize>,
) -> Vec<Uuid> {
    let mut after = before.to_vec();
    after.sort_by_key(|id| rank.get(id).copied().unwrap_or(usize::MAX));
    after
}

#[cfg(test)]
mod pool_order_tests {
    use super::reorder_by_rank;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn rank(ids: &[Uuid]) -> HashMap<Uuid, usize> {
        ids.iter().enumerate().map(|(i, id)| (*id, i)).collect()
    }

    #[test]
    fn reorders_members_by_global_rank() {
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        // 当前绑定 [a,b,c], 全局顺序 [c,a,b] → 重排为 [c,a,b]
        let out = reorder_by_rank(&[a, b, c], &rank(&[c, a, b]));
        assert_eq!(out, vec![c, a, b]);
    }

    #[test]
    fn keeps_membership_and_sinks_unknown_ids() {
        let (a, b, c, x) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        // 全局顺序只提到 c,a,x (x 未绑定) → 已绑定成员仍只有 {a,b,c}, b 落末尾, 不引入 x
        let out = reorder_by_rank(&[a, b, c], &rank(&[c, a, x]));
        assert_eq!(out, vec![c, a, b]);
        assert!(!out.contains(&x), "不得改变成员集");
    }

    #[test]
    fn no_change_when_already_in_order() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let before = vec![a, b];
        assert_eq!(reorder_by_rank(&before, &rank(&[a, b])), before);
    }
}
