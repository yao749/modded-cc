# 方案：池专属的「添加订阅」页面

> 2026-10-28 · 需求：点击 池管理 → 添加订阅 时，不再跳转复用的经典订阅向导，
> 而是进入一个为能力池范式设计的新页面。

---

## 一、现状与问题

### 1.1 现在的流程

```
/pool ──添加订阅──▶ /subscriptions/new?from=pool   （复用经典两步向导）
                       步骤1: 选厂商 / 填 Key / OAuth        ← 与经典页完全相同
                       步骤2: ModelSlotPicker 四槽 + effort  ← 四槽范式的 UI！
                              (只多一条 poolModeHint 提示文字)
                       创建后 → 回 /pool?edit=<id> 再打开池编辑器
```

### 1.2 设计错位

经典向导的步骤 2 是**四槽范式**的配置面板（model-fable/opus/sonnet/haiku +
slot effort），与池订阅毫无关系——池订阅真正需要配的是**能力分类 → 模型链**
（vision / llm / multimodal / speech / embedding）。用户被迫穿过一个不相关的
页面，创建后还要再跳一次才能开始配池。

### 1.3 ⚠️ 合并遗留回归（必须先修）

v6.5.0 合并时 `CreateSubscriptionInput` 采用了上游版本，**丢掉了
`routing_group` 字段**。后端 4 条创建路径全部硬编码 `"classic"`：

- `commands/subscriptions.rs:398`（template 分支）
- `commands/subscriptions.rs:462`（custom 分支）
- `commands/oauth.rs:167`（ChatGPT OAuth）
- `commands/oauth.rs:443`（Kiro OAuth）

前端 `types.ts` 里字段还在、也还在发送，但被后端静默丢弃——**现在从池入口
创建的订阅实际都进了 classic 组**，`/pool/subscriptions`（按 `useSubscriptions("pool")`
过滤）永远看不到它们。这个回归不修，新页面存的数据照样是错的。

---

## 二、目标设计

### 2.1 新页面：`/pool/new`（`PoolSubscriptionNew.tsx`）

独立路由、独立页面，**不做整页复用**，但叶子组件继续共享
（`ChatGptOAuthDialog` / `KiroAuthDialog` / `ModelPoolPicker` / `ProviderLogo` /
`Select` 等本来就是独立组件）。

```
/pool ──添加订阅──▶ /pool/new
   步骤1「接入」: 厂商选择(模板/自定义) + API Key + url 参数 + 名称 + OAuth
   步骤2「能力配置」: ModelPoolPicker 五个能力分类的模型链
                     + 自动填充(N) + 刷新模型列表 + 测试连接
                     （没有四槽、没有 effort、没有 poolModeHint——页面本身就是池的）
   完成 → save_model_pool → 回 /pool
```

### 2.2 两步的内容

**步骤 1（接入）**——精简版，只保留池场景需要的东西：

- 厂商下拉（沿用 first_party / second_party / aggregator 分组 + 能力标签）
- API Key 输入 / ChatGPT OAuth / Kiro OAuth（对话框组件直接复用）
- 自定义协议表单（base_url / messages_path / 鉴权头）
- 显示名（自动生成 + 可改）
- url 参数（模板 endpoint 需要的变量）
- **不含**：onboarding 引导分支、导入配置分支（那是经典首次启动流程的）

**步骤 2（能力配置）**——池范式的核心：

- `ModelPoolPicker`：按 5 个能力分类逐个配模型链（拖拽调优先级）
- 「自动填充 (N)」：把模型发现列表中推断匹配当前能力的模型一键入链
- 「刷新模型列表」：没拉到列表时重新发现
- 底部按钮：`完成`（保存能力链）/ `跳过`（链留空，稍后去 /pool 配）

### 2.3 创建时序（create-first，与现有 OAuth 路径一致）

`ModelPoolPicker` 和模型发现都依赖已存在的 `subscriptionId`
（`refresh_model_list` 要拿订阅的 endpoint + api key 去探测），所以：

```
步骤1 点「下一步」
  → create_subscription (占位 slots="(pending)", routing_group="pool")   ← 一次性
  → refresh_model_list 拿真实模型列表
  → 进入步骤2 (此时订阅已存在, ModelPoolPicker 可用)
步骤2 点「完成」
  → save_model_pool (能力链)
  → navigate("/pool")
步骤2 点「跳过」/「上一步」
  → 订阅保留 (与现有 OAuth 路径的语义一致: 创建即存在)
  → 矩阵页该订阅的空能力格显示"待配置"
```

占位 slots 沿用 OAuth 路径的 `uniformSlots("(pending)")`：这个值永远不会被
任何客户端当作模型名请求，等效于该订阅不参与四槽分发。

---

## 三、改动清单

### 3.1 后端（P0，先做）

| 文件 | 改动 |
|---|---|
| `commands/subscriptions.rs` | `CreateSubscriptionInput` 加回 `pub routing_group: Option<String>`；template / custom 两分支改 `input.routing_group.unwrap_or_else(\|\| "classic".into())` |
| `commands/oauth.rs` | ChatGPT / Kiro 两条创建路径同样取 `input.routing_group`（前端 OAuth 调用已在发送该字段） |
| 测试 | 补 2 个用例：缺省 → classic；传 "pool" → 落库 pool |

### 3.2 前端

| 文件 | 改动 |
|---|---|
| `src/routes/PoolSubscriptionNew.tsx` | **新增**，预计 ~450–550 行（精简步骤1 + ModelPoolPicker 步骤2 + 三条创建路径的时序编排） |
| `src/App.tsx` | 注册 `<Route path="/pool/new" ...>` |
| `src/routes/Pool.tsx` | 「添加订阅」按钮改跳 `/pool/new`（去掉 `?from=pool`） |
| `src/routes/SubscriptionNew.tsx` | 删除 `from=pool` 全部分支（`fromPool` 变量、`routing_group` 发送、`poolModeHint` 两处、`afterCreateNavigate` 的池回流），回归纯经典页 |
| `src/i18n/locales/{zh,en,ja}.json` | 增 `pool.new.*`（标题/步骤名/能力配置区文案/完成/跳过）；删 `subscriptionNew.poolModeHint` |

### 3.3 分发隔离（可选，建议做）

分组制的设计口径是「两个组的订阅互不连通」，但目前 pipeline **不看
`routing_group`**——隔离事实上靠占位 slots 撑着：

- 能力池分发（`list_pool_models`）按"有无池条目"筛选：天然安全 ✅
- 四槽分发不筛组：pool 组订阅若 slots 是 `(pending)` 不会被真实请求命中，
  但 classic 组订阅**配了池条目后也能被能力池调用**（共享底座行为）❌ 与分组制矛盾

建议补一条对称规则（`pipeline.rs` 候选筛选处）：
- 四槽 VM 分发 → 只考虑 `routing_group = "classic"`
- 能力 VM 分发 → 只考虑 `routing_group = "pool"`

两个 query 各加一个条件即可，测试好写。**不做也能跑**（现状已如此），但做了
分组制的语义才闭环。

---

## 四、实施顺序

1. **P0**：后端 routing_group 回归修复 + 测试（~30 分钟级，独立可验证）
2. 新页面 `/pool/new` + 路由 + 入口改跳 + i18n
3. 经典页摘除 `from=pool` 分支
4. （可选）pipeline 分组隔离 + 测试
5. 全量验证：`cargo test --lib` + `tsc --noEmit` + 手动走「API Key」和
   「ChatGPT OAuth」两条路径，确认落库组别与矩阵页显示

## 五、风险与取舍

- **步骤1 逻辑重写而非抽取**：经典页 1399 行、步骤1 与 onboarding/import
  深度耦合（18 处 onboarding 分支），抽取共享组件的重构风险大于重写精简版的
  成本。重复的 ~200 行集中在厂商下拉和自定义表单，属于可接受的稳态重复。
- **创建即存在**：步骤1 完成后订阅已落库，「上一步/跳过」不会删除它——与现有
  OAuth 路径语义一致，避免"删除刚授权的 OAuth 账号"这种破坏性隐式操作。
- **经典页行为零变化**：摘除 `from=pool` 后经典页对老用户完全一致。
