-- 订阅路由分组: classic = 经典四槽组, pool = 能力池组.
-- 两个组的订阅互不连通: 经典池只显示 classic, 池管理页只显示 pool.
-- 默认 classic 兼容所有老订阅.
ALTER TABLE subscriptions ADD COLUMN routing_group TEXT DEFAULT 'classic';

-- 确保已有行得到默认值
UPDATE subscriptions SET routing_group = 'classic' WHERE routing_group IS NULL;
