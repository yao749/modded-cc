-- Jev (System One) 支持
-- endpoint_protocol: 创建订阅时快照所选端点的协议, 之后不可变 (换端点不许改协议)
-- model_slot_jev: model-jev 的槽位, 空串 = 透传客户端 model
ALTER TABLE subscriptions ADD COLUMN endpoint_protocol TEXT NOT NULL DEFAULT 'messages';
ALTER TABLE subscriptions ADD COLUMN model_slot_jev TEXT NOT NULL DEFAULT '';
