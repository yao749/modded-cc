# 方案：池订阅故障转移「纯丝滑」模式（变体 B）

> 状态：**已实现**（2026-10-30 复核）。
>   - `retry.rs:99` 与 `state_machine.rs:338` 的 B 变体改动已随 `a715fa4` 提交；
>   - 「池模式（chain）下任何失败立即 `break 'sub_loop` 切下一订阅 + 统一 demote」的 pipeline 改动在工作区，**尚未提交**。
> 关联：`docs/pool-subscription-page-plan.md`
> 决策：用户选定 **B** —— 401/403 不进 `AuthFailed` 终态、不显示「凭证失效」、不进冷却；Key 坏了也当普通失败转移，一声不吭。统一策略仍是「任何报错 → 模型级降序 + 切换订阅」。

> **魔改声明**：本文档所记述的 cc-router 为 **cc** 的二次开发分支（魔改 / fork），原作者 **cc**（[@//cc/](https://github.com/finch-xu/cc-router)）。本人仅为二次开发者，并非原作者。

---

## 0. 目标与原则

- **统一故障转移**：任何非 2xx 错误 → 模型级降序（`priority=链尾，落库`）+ 切换到下一个候选（订阅级或模型级），当前请求**永不立即报错卡死**，直到成功或撞 `MAX_TOTAL_ATTEMPTS=15`。
- **B 变体特例**：401/403 不进 `AuthFailed`、不显示「凭证失效」、不进冷却；凭证错误被静默吞掉，仅由模型级降序承担隔离。

---

## 1. 现状还原（精确代码位置）

- `proxy/retry.rs:99`：`401 | 403 => ShouldRetry::Yes(RetryReason::AuthFailed)`。
  - 问题：被分类为 `Yes` 后，pipeline 走 `continue`（试同订阅余下模型），而非 `break 'sub_loop`（切下一订阅）。与上方 L89 注释「401/403 → 切下家」自相矛盾。
- `subscription/state_machine.rs:338-342`：`401 | 403 => SubscriptionState::AuthFailed`（终态，`last_error=AuthFailed`，UI 显示「凭证失效」；且 `(AuthFailed, _)` 唯一出口是 `UserUpdateKey`，不自动恢复）。
- `proxy/pipeline.rs` 9 处失败分支（含 1614、789、911、1044、1176、1307、1648、1716 等）统一流程：
  1. `demote_failed_subject(...)` —— 模型降序落库（已实现）
  2. `apply_with_hint(Event::HttpStatus(status))` —— 喂状态机（当前 401/403 会进 `AuthFailed`）
  3. 按 `classify_response(...)` 决定 `continue`（`Yes`，试同订阅余下模型）/ `break 'sub_loop`（`No`，chain_mode 下切下一订阅）
- **结论**：pipeline 已正确做「降序 + 切订阅」，但 401/403 因 `retry.rs` 判 `Yes` 而**没切订阅**、又因 `state_machine` 判 `AuthFailed` 而**拉黑整条订阅**。B 要修的正是这两点，且**几乎不用动 pipeline**。

---

## 2. 改动清单（最小、精确）

### 2.1 `proxy/retry.rs`

- **L99** 改：
  ```rust
  // 改前
  401 | 403 => ShouldRetry::Yes(RetryReason::AuthFailed),
  // 改后
  401 | 403 => ShouldRetry::No,
  ```
  - 效果：pipeline 对该状态走 `else` 分支 → chain_mode 下 `break 'sub_loop`（立即切下一订阅），不再在同订阅内空转试其余模型（共用坏 Key 试也是白试）。
- **L89** 注释「401/403 → 切下家,触发 AuthFailed 状态」→「401/403 → 切下家,不进 AuthFailed(纯丝滑),由模型级降序承担隔离」。
- **L122-131** 测试 `auth_failed_retries_with_auth_reason`：改为断言 401/403 返回 `ShouldRetry::No`（或重命名为 `auth_failed_switches_subscription`）。
- **L15** `RetryReason::AuthFailed` 枚举变体：grep 确认仅 retry.rs 内部引用（pipeline 只匹配 `Yes(_)`，不按 reason 分支），删除安全。

### 2.2 `subscription/state_machine.rs`

- **L338-342** 改：401/403 不再返回 `AuthFailed`，与 400 同处理（返回 `Healthy`，且不写 `last_error=AuthFailed`）：
  ```rust
  401 | 403 => {
      // 纯丝滑: 凭证类错误不当终态, 由 pipeline 的模型级降序承担隔离;
      // 订阅保持 Healthy, UI 不显示「凭证失效」, 不进冷却。
      SubscriptionState::Healthy
  }
  ```
  - 不调 `bump_transient` → 不累计 `consecutive_errors`、不进 `TransientError` 冷却。
- **L479、L491** 断言 `AuthFailed` 的用例改为断言 `Healthy`（或删除）。
- 检查 `rt.row.is_auth_failed` 字段：因不再经 `AuthFailed` 转移，该标志保持 `false`，UI 自然不显示失败。确认无其它代码依赖该标志做快照跳过（B 下应允许继续参与轮换，靠 model-level demote 沉底）。

### 2.3 `proxy/pipeline.rs`

- **无需改动**。现有结构已满足：L1614 降序 → L1615 apply（现 `Healthy`）→ L1625-1632 因 `No` 而 `break 'sub_loop`。9 处分支统一受益。
- 一致性检查（可选）：OAuth 分支（L729-733）用 `retryable` 布尔决定 `continue`/`break`；确认其 401 计算随 `retry.rs` 改变（若由 `classify_response` 推导则已自动生效；若硬编码则需同步改为非 retryable）。

---

## 3. 行为推演

- **稳态（有正常订阅）**：请求先试正常订阅的模型 → 命中即返回；坏 Key 订阅的模型经降序沉到链尾，稳态下根本不会被实际打到（成功在前面就返回），零额外延迟、零报错、UI 全绿。
- **坏 Key 修复**：用户改 Key 保存 → 该订阅模型仍沉在链尾；下次被选中且 200 即恢复服务。建议（可选）保留「重置优先级」按钮让被降序模型归位（现有机制）。
- **全部订阅都坏**：逐模型降序 + 切下一订阅，撞 `MAX_TOTAL_ATTEMPTS=15` 后返回最后一次错误（已尽力转移，而非静默吞掉）。
- **可见性保留**：pipeline 已 `emit_attempt_finished(...,false)` + `request_log_tx` 落请求日志，401/403 失败仍记录在日志（开发者/排障可见），只是不进订阅状态、不弹 UI。

---

## 4. 风险与权衡（B vs A）

- **故障静默**：Key 过期/失效无 UI 提示，用户可能长期不知有一条废订阅在「占位」。排障需靠请求日志/响应体。
- **多订阅同时坏**：界面全绿，但请求在撞 15 次后才报错（延迟报错而非立即红字）。
- **缓解**：日志层已保留 401/403 记录（不进 UI 状态），事后排障可行。

---

## 5. 测试要点（落地时补）

- `retry.rs` 单测：401/403 → `No`；429 仍 `Yes(RateLimited)`；5xx 仍 `Yes(ServerError)`；400 → `No`。
- `state_machine` 单测：注入 `HttpStatus(401)`/`(403)` → 断言状态保持 `Healthy`、`is_auth_failed=false`、不累计 `consecutive_errors`。
- `pipeline` 集成/单测：注入坏 Key 订阅（401）+ 正常订阅，断言①坏订阅其余模型未被尝试（请求直接落到下一订阅）②坏订阅状态未变 `AuthFailed` ③坏模型被 demote 到链尾。
- 回归：5xx 连 3 次仍进 `TransientError`；429 仍用 `Retry-After` 冷却；`MAX_TOTAL_ATTEMPTS` 仍兜底返回错误。

---

## 6. 不在本次范围

- 不改 429/5xx 的冷却（保留为未来请求优化，不阻塞当前请求）。
- 不改 demote 落库/排序逻辑本身。
- 不做「坏 Key 一次性降序整条订阅所有模型」的优化（可选增强，非必需）。
- 不新增 UI 信号（B 的本质）。
