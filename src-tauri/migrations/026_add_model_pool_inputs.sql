-- per-model 输入模态声明 (借鉴 DSH 的 input: [text, image]).
-- 一个模型可以支持多种输入模态: text / image / audio.
-- NULL 表示"未声明" (路由时当作 [text] 兜底).
-- JSON 数组格式: 如 ["text","image"].
ALTER TABLE subscription_model_pool ADD COLUMN supported_inputs TEXT DEFAULT NULL;

-- 已有行自动填默认值 [text] (迁移后兼容旧数据)
UPDATE subscription_model_pool SET supported_inputs = '["text"]' WHERE supported_inputs IS NULL;
