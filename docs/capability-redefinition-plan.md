# 方案：能力分类语义重定义（vision = 媒体生成 / multimodal = 全模态翻译）

> 状态：讨论稿（仅方案，未改代码）
> 关联：`docs/pool-subscription-page-plan.md`、`docs/failover-smooth-plan.md`
> 决策：用户重定义两个能力分类的语义 ——
> - **vision**：由「图像生成/编辑/理解」收敛为 **文生图 + 文生视频（媒体生成）**
> - **multimodal**：由「图文混合输入输出」收敛为 **全模态翻译模型（MT-uni：Machine Translation unified）** —— 文本 / 文档 / 图片 / 音频四类输入，什么都能翻译，并保留原排版

> **魔改声明**：本文档所记述的 cc-router 为 **cc** 的二次开发分支（魔改 / fork），原作者 **cc**（[@//cc/](https://github.com/finch-xu/cc-router)）。本人仅为二次开发者，并非原作者。

---

## 0. 核心原则（先定调，省掉 80% 的迁移成本）

**分类标识符 `vision` / `multimodal` 的字符串值保持不变，只改语义、展示标签、`supported_inputs` 默认值与路由行为。**

理由与收益：
- `subscription_model_pool.capability` 的 `CHECK` 约束（`migrations/025`）仍是 `('vision','llm','multimodal','speech','embedding')`，**无需新 migration 改约束**。
- `VirtualModelName` / `CapabilityCategory` 的 serde 值、`model-vision` / `model-multimodal` 对外虚拟名全部不变 → 已部署用户的现有池配置、已保存的订阅绑定、备份文件**零破坏**。
- 改动收敛在：注释、`label()`、前端 `CAPABILITY_LABEL`、i18n、`default_inputs_for_capability`、路由识别逻辑。这是"改名 + 换路由语义"，不是"改数据结构"。

> 一句话：**标识符不动，语义换血。**

---

## 1. 新语义定义

| capability | 新定位 | 对外虚拟名 | 输入 | 输出 |
|---|---|---|---|---|
| `vision` | **媒体生成**：文生图 + 文生视频 | `model-vision` | 文本提示词（可选参考图） | image / video |
| `multimodal` | **全模态翻译 MT-uni + 图片理解**：保留原排版 | `model-multimodal` | text / document / image / audio | 译文（+ 原版式回填）/ 图片理解结果 |
| `llm` | 大语言（不变） | `model-llm` | text | text |
| `speech` | 语音 TTS/ASR（不变） | `model-speech` | text / audio | audio / text |
| `embedding` | 向量嵌入（不变） | `model-embedding` | text | vector |

### 1.1 四类输入的具体口径（multimodal）

| 输入类 | 格式 | 说明 |
|---|---|---|
| 文本 | 纯文本 / Markdown / TXT | 直接翻译 |
| 文档 | PDF / Word(.docx) / PPT(.pptx) / Excel(.xlsx) / HTML / Markdown / TXT | 解析后翻译，**保留原排版回填** |
| 图片 | png/jpg/webp…（含文字） | OCR → 翻译 → 版面回填 |
| 音频 | mp3/wav/m4a… | ASR → 翻译（语音翻译） |

### 1.2 原 vision 的「看图 / OCR」去向：整体归入 multimodal

原 `vision` 的注释是「图像生成/编辑/**理解**」，`store.rs` 默认输入是 `["text","image"]`，实际做的是**看图理解**。`vision` 收敛为「媒体生成」后，这部分能力**整体迁入 `multimodal`**：

- **看图问答 / 视觉推理（VQA）** → 归 `multimodal`：它的 `supported_inputs` 本就含 `image`，天然承接；走 `chat/completions` + `image_url`，与「图片翻译」共用同一入口。
- **图片里的文字翻译（OCR 翻译）** → 归 `multimodal`：输出是译文，属于翻译任务。
- 结论：`vision` 只做"生成"；`multimodal` 同时承担「图片理解 + 全模态翻译」；`llm` 保持纯文本不变（见 §4.1），不承接看图。

---

## 2. 现状还原（精确代码位置）

### 2.1 后端

| 位置 | 现状 | 需改 |
|---|---|---|
| `src-tauri/src/virtual_model/model.rs:25` | `/// 视觉模型池 (图像生成/编辑/理解)` | → `媒体生成池 (文生图 / 文生视频)` |
| `src-tauri/src/virtual_model/model.rs:31` | `/// 多模态模型池 (文本+图像混合输入输出)` | → `全模态翻译池 (MT-uni: 文本/文档/图片/音频 → 译文, 保留原排版; 兼图片理解)` |
| `src-tauri/src/virtual_model/model.rs:64-72` `label()` | `Vision => "视觉模型"`、`Multimodal => "多模态模型"` | → `"图像/视频生成"`、`"全模态翻译"` |
| `src-tauri/src/subscription/store.rs:730-736` `default_inputs_for_capability` | `"vision" \| "multimodal" => [text, image]` | 拆开：vision → `[text]`；multimodal → `[text, document, image, audio]` |
| `migrations/025_add_model_pool.sql:5-7` | 注释「vision 图像生成/编辑/理解」「multimodal 多模态」 | 仅改注释（CHECK 不动） |
| `src-tauri/src/subscription/store.rs:724-726` 注释 | supported_inputs 示例 | 补充 `document` 模态 |

### 2.2 前端

| 位置 | 现状 | 需改 |
|---|---|---|
| `src/types.ts:301-306` `CapabilityCategory` | 5 字符串联合 | **不变** |
| `src/types.ts:309-315` `CAPABILITY_LABEL` | 视觉模型 / 多模态模型 | → `图像/视频生成` / `全模态翻译` |
| `src/types.ts:346` `InputModality` | `"text" \| "image" \| "audio"` | **加 `"document"`** |
| `src/types.ts:348-361` `defaultInputsForCapability` | vision/multimodal 都 `[text,image]` | 拆开，同 §4.1 |
| `src/components/ModelPoolPicker.tsx:21` `CAPABILITIES` | 数组 | 不变（标识符不变） |
| `src/components/ModelPoolPicker.tsx:437` 文案 | 「能力分类下的模型，将以 model-xxx 暴露」 | 文案补充新语义 |
| `src/components/PoolParadigmCard.tsx:31` `CAP_ORDER` | 数组 | 不变 |
| `src/routes/Pool.tsx:18` `CAPABILITIES` | 数组 | 不变 |
| `src/routes/SubscriptionEdit.tsx:64` `POOL_TEST_ORDER` | 数组 | 不变 |
| `src/components/stats/VmShareDonut.tsx:18-20` 配色 | `model-vision` / `model-multimodal` 颜色键 | 键不变，颜色可保留 |

### 2.3 i18n（四语言）

| 键 | 现状（zh） | 需改 |
|---|---|---|
| `vm.capability.vision.purpose` | `看图` | → `文生图 / 文生视频` |
| `vm.capability.vision.purposeEn` | `Image understanding` | → `Image / video generation` |
| `vm.capability.vision.label` | `视觉模型 · 看图 / OCR` | → `媒体生成 · 文生图 / 文生视频` |
| `vm.capability.multimodal.purpose` | `多模态` | → `全模态翻译` |
| `vm.capability.multimodal.purposeEn` | `Text + image` | → `Omni-modal translation` |
| `vm.capability.multimodal.label` | `多模态模型 · 图文混合` | → `全模态翻译 · 保留原排版` |
| `settings.routing.slotsDesc` / `poolDesc` | 列出能力池名 | 保留，语义可补一句 |

- 四语言 `zh / zh-Hant / en / ja` 各改 6 键；zh-Hant 走 opencc `s2twp` + `docs/zh-Hant-glossary.md` 术语表（"调度"不译"排程"）。
- **注意**：现存 `en.json:1000` 的 `vm.capability.multimodal.purpose` 值是日文 `マルチモーダル`（历史残留），本次一并修正。

### 2.4 文档

- `cc-router使用教程.md` 第十三节「能力池」：更新 vision / multimodal 的说明与示例；Trae 接入示例仍用 `model-llm`，不受影响。

### 2.5 备份

- `src-tauri/src/backup/format.rs` 的 `ExportModelPoolEntry.capability` 是字符串透传，**结构不变**；但导入旧备份时 vision 条目的语义已变 → 需在 UI 给提示（见 §6）。

---

## 3. 路由设计

### 3.1 vision（媒体生成）

现状：**代理层没有生成端点**（全仓库 `images/generations` / `videos/generations` 零匹配），只有 `chat/completions` 与 `responses`。必须新增：

| 端点 | 形态 | 路由 |
|---|---|---|
| `POST /v1/images/generations` | 同步，返回 `data[].url` / `b64_json` | model=model-vision → 过滤 `output=image` 的条目的链 |
| `POST /v1/videos/generations` | 异步：提交返回 `task_id` → 轮询 `GET /v1/videos/generations/{id}` | model=model-vision → 过滤 `output=video` 的条目 |

**池内如何区分图 / 视频（已定：方案 a —— 新增 `output_modality` 列）**

`vision` 池同时容纳文生图与文生视频模型，二者**输入都是文本提示词**，仅输出不同，因此不能靠 `supported_inputs` 区分，必须新增输出维度。

- **字段**：`output_modality TEXT NOT NULL DEFAULT 'image'`，取值 `'image' | 'video'`；仅 `vision` 池使用，其余 capability 恒为默认值、路由时忽略。
- **路由**：
  - `POST /v1/images/generations` → 只在 `capability='vision' AND output_modality='image'` 的条目链里选模型；
  - `POST /v1/videos/generations` → 只在 `... output_modality='video'` 的条目链里选模型。
  - 两类条目可混放在同一 `vision` 池、互不干扰；某类为空时直接返回 4xx（「该分类下没有可用的图像/视频生成模型」），不空转重试。
- **UI**：池编辑页 `vision` 池的条目行增加「输出类型：图像 / 视频」下拉；其他池隐藏该控件，恒写 `image`。
- **备份**：`ExportModelPoolEntry` 增加 `output_modality` 字段（可选，缺省 `image`）；导入旧备份时缺失字段按 `image` 处理，向后兼容。
- **i18n**：新增 `pool.outputModality.image` / `.video` 及标签键，四语言同步。

### 3.2 multimodal（全模态翻译）

翻译请求形态不统一，按输入类分路：

| 输入类 | 端点策略 | 备注 |
|---|---|---|
| 文本 | 复用 `POST /v1/chat/completions`，model=model-multimodal | 最简单，先通 |
| 图片 | 复用 `chat/completions` + `image_url` | OCR+翻译 / 看图理解同入口，由上游模型完成 |
| 文档 | 新增 `POST /v1/translations`（multipart/form-data 上传） | 需解析 + 回填，见 §3.3 |
| 音频 | 复用 `chat/completions` 的 audio 输入，或 `/v1/audio/translations` | Whisper 式仅英译，能力有限 |

- 路由过滤：`detect_request_inputs` 扩展识别 `document`（multipart + 扩展名/Content-Type 判定 pdf/docx/pptx/xlsx/html/md/txt）与 `audio`，按池条目 `supported_inputs` 过滤不兼容模型。
- 现有 `pipeline.rs:495-518` 的 `skipped_input_mismatch` 逻辑直接复用，只需补 `document` 分支。

### 3.3 「保留原排版」的边界（必须写清，避免预期错位）

**路由层只负责「选模型 + 转发 + 回传」，不负责版面还原。** 排版保留取决于：
- 上游服务能力（如支持 docx/pdf 原位回填的翻译 API），或
- 本地后处理（引入文档解析库：`pdf` 解析、`docx`/`pptx`/`xlsx` 的 OOXML 读写、版面重建）。

若要求"严格保留原排版"，cc-router 侧需新增一条**重管线**：文件解析 → 分块翻译 → 回填 → 重新打包。这是 P3 阶段最重的部分，工作量远超路由本身，需单独立项评估，**不在本方案的"改名"范围内**。

---

## 4. supported_inputs 新默认值

### 4.1 默认值调整

| capability | 旧默认 | 新默认 |
|---|---|---|
| `vision` | `["text","image"]` | `["text"]`（提示词；输出不算输入） |
| `multimodal` | `["text","image"]` | `["text","document","image","audio"]` |
| `llm` | `["text"]` | **不变** `["text"]`（看图理解已归 multimodal） |
| `speech` | `["text","audio"]` | 不变 |
| `embedding` | `["text"]` | 不变 |

### 4.2 `InputModality` 扩展

- 后端：`store.rs` 的 supported_inputs 是 `Vec<String>`，新增 `"document"` 无需改类型，只需在识别/过滤处纳入。
- 前端：`src/types.ts:346` `InputModality` 加 `"document"`；`ModelPoolPicker` 的模态勾选 UI 增加「文档」项。

---

## 5. 数据兼容与迁移

### 5.1 旧 vision 条目的语义漂移

旧 `vision` 条目（如 `glm-v`）实际是**看图模型**，新语义下 `vision` 变成"生成"。直接改语义会让这些条目被误当成文生图模型。

迁移策略（三选一，建议 c）：

| 方案 | 做法 | 代价 |
|---|---|---|
| a 静默 | 不动数据，只改 label | 用户旧配置语义错位，无感知 |
| b 强迁 | 新 migration 把 `vision` 条目 `supported_inputs` 从 `[text,image]` 改为 `[text]` | 简单，但仍不解决"模型本身是看图模型" |
| c **提示 + 可选一键迁移** | 不改数据；在池编辑页对 `vision` 条目显示提示「vision 已改为图像/视频生成，若该模型仅支持看图理解，请迁至全模态翻译」并提供一键移动 | 体验最好，需前端一个小交互 |

→ 推荐 **c**。

### 5.2 旧备份导入

- `backup/import.rs` 已按 `!new_ids.contains` 只导入新增池；导入后若含 `vision` 条目，UI 显示同 §5.1 的提示。

### 5.3 `output_modality` 新列

- 新 migration：`ALTER TABLE subscription_model_pool ADD COLUMN output_modality TEXT NOT NULL DEFAULT 'image';`
- 取值 `'image' | 'video'`，**应用层校验**（SQLite `ALTER TABLE ADD COLUMN` 带 CHECK 对已有行有兼容约束，不加 DDL 级 CHECK，避免与默认值冲突）。
- 旧行自动取 `'image'`，向后兼容；旧备份导入同样缺省 `image`。
- 可选索引（生成路由取链用）：`CREATE INDEX idx_pool_sub_cap_out_pri ON subscription_model_pool(subscription_id, capability, output_modality, priority);`

---

## 6. 落地分阶段（按风险从低到高）

| 阶段 | 内容 | 影响面 | 风险 | 状态 |
|---|---|---|---|---|
| **P0 语义层** | 注释、`label()`、前端 `CAPABILITY_LABEL`、i18n 四语言、教程文档 | 纯文本 | 零 | ✅ **已完成**（2026-10-30，未提交） |
| **P1 输入模态** | `InputModality` 加 `document`；`default_inputs_for_capability` 调整；`detect_request_inputs` 识别 document；`skipped_input_mismatch` 补分支 | 路由识别 | 低 | ✅ **已完成**（2026-10-30，未提交） |
| **P2 vision 生成** | 新增 `images/generations`（同步）→ `videos/generations`（异步 + 任务表）；新增 `output_modality` 列与池内过滤 | 新端点 + 新列 | 中 | ⏸ 未开始（仅 `output_modality` 设计已定案） |
| **P3 multimodal 翻译** | 先复用 `chat/completions`（文本/图片/音频），再新增 `/v1/translations`（文档 multipart + 排版回填） | 新端点 + 文档管线 | **高** | ⏸ 未开始 |
| **P4 迁移提示** | 池编辑页 vision 条目提示 + 一键迁移 | 前端交互 | 低 | ⏸ 未开始 |

**P0 / P1 已完成的代码落点**（均在未提交的工作区里）：

| 落点 | 改动 |
|---|---|
| `src-tauri/src/virtual_model/model.rs` | `Vision` 注释 →「媒体生成池 (文生图 / 文生视频)」；`Multimodal` →「全模态翻译池 (MT-uni…兼图片理解)」；`label()` →「图像/视频生成」「全模态翻译」 |
| `src-tauri/migrations/025_add_model_pool.sql` | 注释按新语义重写（vision=媒体生成、llm=纯文本、multimodal=全模态翻译） |
| `src-tauri/src/subscription/store.rs` | `default_inputs_for_capability`：`vision → ["text"]`、`multimodal → ["text","document","image","audio"]`（llm 保持 `["text"]` 不动） |
| `src-tauri/src/proxy/pipeline.rs` | `detect_request_inputs` 加 document 识别（block 类型 + URL 扩展名 / data URI mime 启发式）；`skipped_input_mismatch` 加 `document` 过滤分支 |
| `src/types.ts` | `CAPABILITY_LABEL` 同步；`InputModality` 加 `"document"`；`defaultInputsForCapability` 同步 |
| `src/i18n/locales/{zh,zh-Hant,en,ja}.json` | `vm.capability.vision.*` / `vm.capability.multimodal.*` 各 3 键改写 |
| `cc-router使用教程.md` | 第十三节：新语义表 + 输入模态说明 + Trae 模型 ID 指引更新 |

> `is_doc_url` 初版有运算符优先级 bug（`&&` 高于 `|`，导致任意 URL 只要含 `msword` 字样就误判为文档），已修正为带括号的 `||` 组合。

**建议**：P2、P3 各自独立提交并单独立项评估。

---

## 7. 风险与待拍板

1. **生成端点是"从零新增"，不是改名** —— `images/generations` 同步链路 + `videos/generations` 异步轮询（需任务状态表、超时、回调），是 P2/P3 的主要工作量。
2. **「保留原排版」路由层无法保证** —— 依赖上游能力或本地文档解析重建管线；若要强保证，需单独立项（见 §3.3）。
3. **看图理解归属** —— 已定归 `multimodal`（与图片翻译共用 `chat/completions` + `image_url` 入口）；`llm` 保持纯文本，`default_inputs_for_capability` 中 llm 不改。
4. **视频生成标准不统一** —— Sora / 可灵 / Runway / Veo 各家 API 形态不同，`videos/generations` 可能需要按 provider 适配，无法一个端点通吃。
5. **旧 vision 条目语义漂移** —— 采用 §5.1 方案 c（提示 + 一键迁移）规避。

---

## 8. 待确认项

- [x] 看图理解归 `multimodal`（已定，§1.2）
- [x] 图 / 视频区分：新增 `output_modality` 列（已定，§3.1 / §5.3）
- [x] P0+P1 立即动手 —— **已实现**（2026-10-30，见 §6 落点表）；P2/P3 仍只保留方案
- [ ] 文档翻译"保留原排版"是否要求本地强保证（决定 P3 是否单独立项，§3.3）

---

## 9. 实现状态小结（2026-10-30）

| 项 | 状态 |
|---|---|
| 语义重定义（注释 / label / i18n / 教程） | ✅ 已改，未提交 |
| `document` 输入模态 + 路由过滤 | ✅ 已改，未提交 |
| 生成端点 `images/generations` / `videos/generations` | ❌ 未实现（全仓库零匹配，代理层目前只有对话端点） |
| `/v1/translations` 文档翻译管线 | ❌ 未实现 |
| `output_modality` 列 + migration | ❌ 未实现（仅设计定案） |
| 旧 vision 条目迁移提示（P4） | ❌ 未实现 |

**重要**：P0/P1 落地只是"语义归位"。把文生图模型放进 `model-vision` 池后，**实际调用仍走 `chat/completions` 对话协议**——
生成能力要真正可用，必须等 P2（生成端点）落地。这一点已同步写进 `cc-router使用教程.md` 第十三节。
