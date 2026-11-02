-- 能力分类模型池: 在现有四槽体系 (fable/opus/sonnet/haiku) 之上新增的可选路由体系.
-- 与旧体系共存, 互不干扰 — 按 VirtualModelName 分道.
--
-- 5 种能力分类 (v6.5 语义重定义):
--   vision     媒体生成 (文生图 / 文生视频)
--   llm        大语言模型 (纯文本生成/对话)
--   multimodal 全模态翻译 (MT-uni: 文本/文档/图片/音频 → 译文, 保留原排版; 兼图片理解)
--   speech     语音 (TTS/ASR)
--   embedding  向量嵌入
--
-- 一条记录 = 某订阅下, 某模型名, 归属于某能力分类, 在该分类池内的优先级位置.
-- 同模型可出现在多个能力分类 (如 gpt-4o 既是 llm 也是 multimodal), 此时写多条.

CREATE TABLE subscription_model_pool (
  id TEXT PRIMARY KEY,
  subscription_id TEXT NOT NULL,
  capability TEXT NOT NULL CHECK (capability IN ('vision','llm','multimodal','speech','embedding')),
  model_name TEXT NOT NULL,
  priority INTEGER NOT NULL DEFAULT 0,    -- 同 capability 下, 0 = 最高优先级
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY (subscription_id) REFERENCES subscriptions(id) ON DELETE CASCADE
);

-- 按订阅查全部池内容 (编辑页展示)
CREATE INDEX idx_pool_sub ON subscription_model_pool(subscription_id);
-- 路由时: 某订阅 + 某 capability → 按优先级排序取首选
CREATE INDEX idx_pool_sub_cap_pri ON subscription_model_pool(subscription_id, capability, priority);
