-- 订阅级定时开关 (能力池订阅用): JSON 数组 [{"days":[1,2],"start":"22:00","end":"08:00"},...],
-- '[]' = 没有定时限制 (恒启用)。窗口内订阅才参与调度, 窗口外等同于手动停用。
ALTER TABLE subscriptions ADD COLUMN enable_windows TEXT NOT NULL DEFAULT '[]';