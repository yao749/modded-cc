# 路由模式门户方案（模式即门户 · UI 分家）

> 状态: 方案待评审 → 实施未开始
> 关联提交: `14664ea`(智能冷却 P0) · `42f60ce`(routing_mode 门禁 S1+S2)

## 0. 目标与裁决

**目标**：四槽路由与能力池路由全局二选一（门禁已实现），UI 按「模式即门户」分家——
经典组页面原样保留，能力池组为新页面，两侧互不干扰，且**用户看到的页面就是生效的路由**。

**已裁决**：
- 门禁保留（选项 A）。代价：两套不能同时用（Claude Code 走 `claude-*` + 自用脚本走 `model-llm` 的组合被禁）。接受。
- 订阅是共享底座（一把 key 一把 key），分家的是**配置面**，不是订阅数据。

## 1. 现状盘点（已就绪，勿重造）

| 能力 | 位置 | 状态 |
|---|---|---|
| `routing_mode` 设置 + 门禁 + `paradigm()` | `settings/model.rs`、`pipeline.rs:196/279` | ✅ |
| 设置页路由模式选择器 | `settings/GeneralTab.tsx`、`useSettingsForm.ts` | ✅ |
| 池条目 API（读/存/测试） | `api/tauri.ts`: `load_model_pool` / `save_model_pool` / `test_pool_connection` | ✅ |
| 池条目编辑器组件 | `components/ModelPoolPicker.tsx`（719 行，现嵌在 SubscriptionEdit） | ✅ 可复用 |
| 侧边导航分组 | `components/layout/Sidebar.tsx` | ✅（`9e59312`） |
| 池页面组（/pool, /pool/models） | `routes/Pool.tsx` / `routes/PoolModels.tsx` | ✅（`9e59312` + 本次修订） |
| 契约过滤（/v1/models、GUI 列表、TUI 按模式） | `handler.rs:810`、`commands/virtual_models.rs`、`tui_contract.rs` | ✅ |
| 备份覆盖 `subscription_model_pool` | `backup/export.rs` / `import.rs` | ✅（`d1c3695`） |
| ModeBanner（只读，无切换按钮） | `components/layout/ModeBanner.tsx` | ✅（本次去掉切换按钮） |
| 池内添加订阅 + 管理订阅区 | `routes/Pool.tsx`、`routes/SubscriptionNew.tsx` | ✅（本次） |
| 池条目即接入（保存时自动同步绑定） | `commands/subscriptions.rs::save_model_pool` | ✅（本次） |
| Attempt Plan 订阅内优先级降级 | `pipeline.rs` + `store.rs:752`(`list_pool_models` 死代码) | ❌ 独立功能，本方案不含 |

## 2. 信息架构

```
经典组（现有页面，零改动）
  /guide  /live-routing  /virtual-models  /subscriptions
  /subscriptions/new  /subscriptions/:id  /request-logs
  /statistics  /receipts  /updates  /settings  /about

能力池组（新增）
  /pool          池总览：订阅 × 能力矩阵 + 优先级链 + 管理订阅区
  /pool/models   能力虚拟模型（调度模式 + 已接入订阅，池专属）
```

**订阅编辑页**：槽位配置区原样保留；池配置区块**移除**，池配置全部收敛到 `/pool`
总览页完成（矩阵单元格点击 → ModelPoolPicker 弹窗）。slots 模式下该页不出现任何池元素。

## 3. 导航分组（Sidebar.tsx）

- `items: NavItem[]` 重构为 `groups: NavGroup[]`（`NavGroup { labelKey?: string; items: NavItem[] }`），
  经典组无组标题，池组带组标题「能力池」。
- 现有 active/dot/badge 逻辑不动，纯数据形状变化。
- 池组在 slots 模式下**可点击**（不置灰）——点进去即门户：页面横幅引导一键切模式。

### 3.1 修订（2026-10-23）：横幅不带切换按钮

ModeBanner 只读，**不含一键切换按钮**（防误触把整站路由切走）。模式切换只走设置页
（settings.routing_mode）。横幅文案三组：
- 池组页面 + pool 模式（弱）：「当前：能力池模式」
- 池组页面 + slots 模式（强）：「当前为四槽模式，这一页的能力池配置没有生效。」
- 经典配置页 + pool 模式（强）：「当前为能力池模式，这一页的四槽配置没有生效。」

### 3.2 修订（2026-10-23）：池内添加订阅 + 管理订阅

- `/pool` 页头新增「添加订阅」→ `/subscriptions/new?from=pool`；创建成功后回流
  `/pool?edit=<subId>` 并自动打开该订阅的池编辑器（SubscriptionNew 的 4 处成功跳转
  统一走 `afterCreateNavigate`）。
- `/pool` 矩阵下方新增「管理订阅」区：列出池内订阅（有任一池条目），支持
  编辑跳转 / 测试连接 / 启停 / 删除（带确认弹窗）。与经典「订阅管理」页数据同源
  （订阅是共享底座），操作实时同步。
- 订阅是共享底座：同一个订阅既可以配四槽也可以配池，两套配置互不干扰。

## 3.3 修订（2026-10-23）：池条目即接入（唯一来源）

`save_model_pool` 保存后自动同步能力虚拟模型绑定：某订阅在某能力下有 ≥1 条**启用**条目
→ 自动加入该能力虚拟模型的 `subscription_ids`；全部禁用/删除 → 自动移除。只动当前订阅
（±），其他订阅的绑定原样。手动在虚拟模型页加的绑定会被下一次池保存覆盖。

因此：
- 池接入的唯一来源是池条目（池总览矩阵），**不需要**单独去虚拟模型页绑定；
- `/pool/models` 是池专属页（5 个能力虚拟模型的调度模式 + 已接入订阅只读列表），
  不再复用经典 VirtualModelsPage；
- 经典 `/virtual-models` 页过滤掉能力名，只呈现四槽；pool 模式下显示未生效提示。


## 4. 模式横幅 ModeBanner（"门户"的核心 UX）

- 组件：`components/layout/ModeBanner.tsx`，路由感知，AppShell 内容区顶部渲染。
- 规则：
  - **池组页面（/pool, /pool/models）**：常显横幅（弱样式）「当前：能力池模式」+ 一键切换到四槽。
  - **pool 模式下访问经典配置页（/virtual-models、/subscriptions/:id）**：强样式警告横幅
    「当前为能力池模式，四槽路由未生效」+ 一键切回 slots。
  - slots 模式下访问池组页面：强样式横幅「当前为四槽模式，能力池路由未生效」+ 一键切到 pool。
  - 主场页面（slots 模式下的经典配置页）无横幅。
- 一键切换 = `PATCH settings { routing_mode }` + 刷新相关 query。后端每请求读设置，**热生效无需重启**。
- 文案三语（zh/en/ja），键前缀 `modeBanner.*`。

## 5. /pool 总览页设计

- **矩阵视图**：行 = 订阅（display_name + provider + 健康状态点），列 = 5 个能力
  （vision / llm / multimodal / speech / embedding）。
- **单元格**：已配置模型数 + 优先级链摘要（如 `glm-4-flash → glm-4.5-air`）+ 启停状态；
  点击打开 ModelPoolPicker 编辑弹窗（组件复用，入口从订阅编辑页改为矩阵单元格）。
- **行操作**：测试连接（复用 `test_pool_connection`）。
- **顶部**：ModeBanner + 能力说明 tooltip + 空状态引导（"选一个订阅开始配置"）。
- **管理订阅区**（矩阵下方）：见 §3.2。
- **数据**：新增后端命令 `list_model_pools()` 一次返回全量池条目（矩阵页需要；
  现有 `load_model_pool` 按订阅查，N 个订阅 = N 次 invoke，前端并行 useQuery 也可但脏检查麻烦）。
  store.rs 加一个 `SELECT ... ORDER BY subscription_id, capability, priority` 即可。

## 6. /pool/models 页设计

**池专属页**（`routes/PoolModels.tsx`，2026-10-23 起不再复用 VirtualModelsPage）：
- 列出 5 个能力虚拟模型名，各自显示：调度模式切换（sequential / round_robin / sticky，
  走现有 `update_virtual_model`）+ 已接入订阅只读列表（chips）。
- 接入由池条目驱动（§3.3），本页不提供手动绑定；空列表时提示去池总览加条目。
- 经典 `/virtual-models` 页只呈现四槽（`VM_ORDER` 过滤掉能力名），pool 模式下显示
  「四槽未生效」提示——两页命名空间彻底分开。

## 7. 契约过滤（门户语义的运行侧保障）

| 位置 | slots 模式 | pool 模式 |
|---|---|---|
| `GET /v1/models`（handler.rs:810） | 6 个经典名 | 5 个能力名 |
| `list_virtual_models`（commands/virtual_models.rs:26） | 6 个 | 5 个 |
| `tui_contract.rs` 虚拟模型清单 | 6 个 | 5 个 |
| backup export `virtual_models` | 全量导出（保数据，不按模式） | 同 |

- 过滤规则：`paradigm() == 当前模式 || paradigm() == Both`（jev 两模式都放行，维持现有 systemone 逻辑）。
- 单测：三处过滤各补模式 × 范式组合的表驱动测试。
- 预期行为说明：pool 模式下 `/v1/models` 看不到 `claude-*` 名——这是门户语义的预期，
  客户端模型发现只应看到当前模式可用的名字。

## 8. P0：备份覆盖 subscription_model_pool

**现状**：`backup/export.rs` 只导 `subscriptions`（含 model_slots）与 `virtual_models`，
`subscription_model_pool` 完全不在备份/恢复范围——备份恢复即丢全部池配置。

- `BackupFile` 加 `model_pools: Vec<PoolBackupItem>`，字段：
  `subscription_id, capability, model_name, priority, enabled, supported_inputs`。
- import：旧备份无此字段 → serde 默认空 vec（import.rs 已有 future_field 宽容解析的先例）。
- 恢复顺序：subscriptions → virtual_models → model_pools（外键依赖 subscriptions）。
- 测试：导出含池条目 / 旧格式无 model_pools 字段可导入 / 恢复后池条目一致。

## 9. 实施顺序（每步可独立提交、独立验证）

| # | 步骤 | 主要文件 | 验证点 |
|---|---|---|---|
| P0 | 备份补 pool 表 | backup/export.rs, import.rs | 导出→恢复→池条目一致；旧备份可导入 |
| P1 | 契约过滤三处 | handler.rs, commands/virtual_models.rs, tui_contract.rs | 单测：模式×范式组合；`curl /v1/models` 实测 |
| P2 | 导航分组 + ModeBanner + 一键切换 | Sidebar.tsx, AppShell.tsx, 新 ModeBanner.tsx | slots 下点池组→横幅引导；切换后 query 刷新 |
| P3 | /pool 总览 + list_model_pools 命令 | 新 routes/Pool.tsx, store.rs, commands, api/tauri.ts | 矩阵渲染；单元格编辑落库；测试连接 |
| P4 | /pool/models | 新 routes/PoolModels.tsx | 5 个能力名绑定管理 |
| P5 | 订阅编辑页池区块移除 + slots 引导 | SubscriptionEdit.tsx | slots 模式无池元素；编辑页回归 |
| P6 | i18n 补齐 + 文档 | 3 locales, README×3, 使用教程 | 三语切换无缺键 |

## 10. 明确不做

- 不删四槽任何代码路径（门禁后面留着，切回即用，数据不丢）。
- 不改调度引擎、状态机、翻译层。
- **Attempt Plan 订阅内降级不在本方案**——它是池的"优先级链真正生效"的功能核心
  （`list_pool_models` 目前是死代码），建议 P3 完成后插队做，或与 P3 并行。
- 不做"两套同时用"的任何妥协（门禁语义不动摇）。

## 11. 风险

1. **P1 过滤的回归面**：`/v1/models` 是客户端模型发现的契约，过滤错会让客户端看不到模型——
   必须有单测 + 两种模式各 curl 实测。
2. **P5 移除池区块**：SubscriptionEdit.tsx 789 行，池相关代码与槽位代码有共享状态
   （如 modal 开关），移除时注意只摘池分支，不动槽位。
3. **热切换的中间态**：切换 routing_mode 后，进行中的请求不受影响（每请求读一次设置），
   新请求按新模式——无需额外处理，但要在文档里写明。
