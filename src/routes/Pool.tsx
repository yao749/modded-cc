import { useEffect, useMemo, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, ChevronUp, ChevronsDown, ChevronsUp, Shuffle, ArrowUpDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ProviderLogo } from "@/components/ProviderLogo";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ModelPoolPicker } from "@/components/ModelPoolPicker";
import { StatusBadge } from "@/components/StatusBadge";
import { api } from "@/api/tauri";
import { useSubscriptions, useSetSubscriptionEnabled, useDeleteSubscription } from "@/hooks/useSubscriptions";
import { VIRTUAL_MODELS_KEY } from "@/hooks/useVirtualModels";
import { useT } from "@/i18n";
import { CAPABILITY_LABEL, type CapabilityCategory, type ModelInfo, type ModelPoolEntry, type SubscriptionDto } from "@/types";

const CAPABILITIES: CapabilityCategory[] = ["vision", "llm", "multimodal", "speech", "embedding"];

export const MODEL_POOLS_KEY = ["model-pools"] as const;

/**
 * 能力分类池总览: 行 = 订阅, 列 = 5 个能力。
 * 单元格显示该订阅在该能力下的优先级链 (按 priority 升序), 点击打开编辑器。
 * 池配置全部收敛到这里, 订阅编辑页不再出现池区块。
 * 「添加订阅」在本页自洽完成创建 (创建完回流到本页并打开编辑器)。
 */
export function PoolPage() {
  const { t } = useT();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const subs = useSubscriptions("pool");
  const queryClient = useQueryClient();
  const pools = useQuery({ queryKey: MODEL_POOLS_KEY, queryFn: () => api.listModelPools() });

  // 池订阅的全局顺序 (等价于 5 个能力模型绑定顺序的合并)
  const orderQuery = useQuery({
    queryKey: ["pool-sub-order"],
    queryFn: () => api.listPoolSubscriptionOrder(),
  });
  const [order, setOrder] = useState<string[] | null>(null);
  useEffect(() => {
    if (orderQuery.data) setOrder(orderQuery.data);
  }, [orderQuery.data]);
  const reorderMut = useMutation({
    mutationFn: (next: string[]) => api.reorderPoolSubscriptions(next),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["pool-sub-order"] });
      queryClient.invalidateQueries({ queryKey: VIRTUAL_MODELS_KEY });
    },
    onError: () => {
      // 乐观更新失败 → 从后端真值恢复, 等 orderQuery 重拉后 useEffect 会同步到 order state
      queryClient.invalidateQueries({ queryKey: ["pool-sub-order"] });
    },
  });

  /** 正在编辑的订阅 */
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState<ModelPoolEntry[]>([]);
  const [saving, setSaving] = useState(false);
  /** 编辑器里的模型发现列表 (供「自动填充」一键导入; null = 还没拉过) */
  const [editModels, setEditModels] = useState<ModelInfo[] | null>(null);
  const [modelsLoading, setModelsLoading] = useState(false);
  /** 管理订阅区: 待删除的订阅 */
  const [deleting, setDeleting] = useState<SubscriptionDto | null>(null);

  const setEnabledMut = useSetSubscriptionEnabled();
  const deleteMut = useDeleteSubscription();

  /** 打开某订阅的池编辑器: 带上该订阅已发现的模型列表 (自动填充/导入用) */
  async function openEditor(subId: string, entries: ModelPoolEntry[]) {
    setDraft(entries);
    setEditing(subId);
    setEditModels(null);
    setModelsLoading(true);
    try {
      const sub = await api.getSubscription(subId);
      setEditModels(sub.model_cache?.models ?? null);
    } catch {
      setEditModels(null); // 拿不到就只靠手动输入 + 刷新按钮
    } finally {
      setModelsLoading(false);
    }
  }

  async function refreshEditorModels() {
    if (!editing) return;
    setModelsLoading(true);
    try {
      const result = await api.refreshModelList(editing);
      // manual_fallback (拉取失败退回手动) 时 models 字段不存在 → 保留原列表
      if (result.kind === "auto") setEditModels(result.models);
    } catch {
      // 刷新失败保留原列表
    } finally {
      setModelsLoading(false);
    }
  }

  // 池的测试按「能力链列表逐个模型」跑, 统一在池编辑页做 (?test=1 进入即测),
  // 这里只负责跳转, 不再用 test_connection 的四槽代表模型 (会测到 key 未开通的模型)。

  const bySub = useMemo(() => {
    const m = new Map<string, ModelPoolEntry[]>();
    for (const e of pools.data ?? []) {
      const list = m.get(e.subscription_id) ?? [];
      list.push(e);
      m.set(e.subscription_id, list);
    }
    for (const list of m.values()) list.sort((a, b) => a.priority - b.priority);
    return m;
  }, [pools.data]);

  // 从「添加订阅」向导回流: /pool?edit=<subId> 直接打开该订阅的编辑器
  const editParam = searchParams.get("edit");
  useEffect(() => {
    if (!editParam || !subs.data) return; // 等订阅列表就绪再处理, 避免竞态
    if (subs.data.some((s) => s.id === editParam)) {
      // 新订阅没有池条目 → draft 为空, 正好让用户直接开始配
      void openEditor(editParam, bySub.get(editParam) ?? []);
    }
    // 清掉 URL 上的 edit, 避免刷新重复弹窗
    navigate("/pool", { replace: true });
  }, [editParam, subs.data, bySub, navigate]);

  function open(subId: string) {
    void openEditor(subId, bySub.get(subId) ?? []);
  }

  async function save() {
    if (!editing) return;
    setSaving(true);
    try {
      await api.saveModelPool(
        editing,
        draft.map(({ subscription_id: _sub, ...rest }) => rest),
      );
      await queryClient.invalidateQueries({ queryKey: MODEL_POOLS_KEY });
      setEditing(null);
    } finally {
      setSaving(false);
    }
  }

  const rows = subs.data ?? [];
  /** 池里的订阅: 有任一能力条目 (含禁用) 即算池成员 —— 供下方「管理订阅」区使用 */
  const poolSubs = useMemo(
    () => rows.filter((s) => (bySub.get(s.id) ?? []).length > 0),
    [rows, bySub],
  );

  const subsById = useMemo(() => {
    const m = new Map<string, SubscriptionDto>();
    for (const s of rows) m.set(s.id, s);
    return m;
  }, [rows]);

  /** 按全局顺序渲染的行; 只含真正绑定到某个能力池的订阅 (即会参与路由的成员) */
  const orderRows = useMemo(() => {
    const seen = new Set<string>();
    const out: SubscriptionDto[] = [];
    for (const id of order ?? []) {
      const s = subsById.get(id);
      if (s && !seen.has(id)) {
        seen.add(id);
        out.push(s);
      }
    }
    return out;
  }, [order, subsById]);

  /** 上下箭头换位: 立即本地生效并落库 (每次交换把完整顺序发给后端) */
  function moveOrder(idx: number, dir: -1 | 1) {
    if (reorderMut.isPending) return;
    if (idx < 0 || idx >= orderRows.length) return;
    const j = idx + dir;
    if (j < 0 || j >= orderRows.length) return;
    const ids = orderRows.map((s) => s.id);
    [ids[idx], ids[j]] = [ids[j], ids[idx]];
    setOrder(ids);
    reorderMut.mutate(ids);
  }

  /** 置顶: 把指定订阅移到第一个位置 */
  function moveToTop(idx: number) {
    if (reorderMut.isPending) return;
    if (idx <= 0) return;
    const ids = orderRows.map((s) => s.id);
    const [moved] = ids.splice(idx, 1);
    ids.unshift(moved);
    setOrder(ids);
    reorderMut.mutate(ids);
  }

  /** 置底: 把指定订阅移到最后一个位置 */
  function moveToBottom(idx: number) {
    if (reorderMut.isPending) return;
    if (idx < 0 || idx >= orderRows.length - 1) return;
    const ids = orderRows.map((s) => s.id);
    const [moved] = ids.splice(idx, 1);
    ids.push(moved);
    setOrder(ids);
    reorderMut.mutate(ids);
  }

  /** 随机打乱全局顺序 */
  function shuffleOrder() {
    if (reorderMut.isPending) return;
    if (orderRows.length < 2) return;
    const ids = orderRows.map((s) => s.id);
    // Fisher-Yates shuffle
    for (let i = ids.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [ids[i], ids[j]] = [ids[j], ids[i]];
    }
    setOrder(ids);
    reorderMut.mutate(ids);
  }

  /** 反转全局顺序 */
  function reverseOrder() {
    if (reorderMut.isPending) return;
    if (orderRows.length < 2) return;
    const ids = orderRows.map((s) => s.id).reverse();
    setOrder(ids);
    reorderMut.mutate(ids);
  }

  /** 某订阅在本池里启用的能力名 (仅用于顺序行副标题) */
  function capsOf(subId: string): string {
    const entries = bySub.get(subId) ?? [];
    const caps = CAPABILITIES.filter((c) => entries.some((e) => e.capability === c && e.enabled));
    return caps.length === 0 ? "—" : caps.map((c) => CAPABILITY_LABEL[c]).join(" / ");
  }

  return (
    <>
      <div className="page-header">
        <h1>{t("pool.title")}</h1>
        <div className="subtitle">{t("pool.subtitle")}</div>
      </div>

      <div className="page-actions" style={{ marginBottom: 12 }}>
        <Button onClick={() => navigate("/pool/new")}>
          + {t("pool.addSubscription")}
        </Button>
      </div>

      {rows.length === 0 ? (
        <div className="card section">
          <div className="card-body text-sm text-muted-foreground">
            <div>{t("pool.empty")}</div>
            <div style={{ marginTop: 6 }}>{t("pool.emptyHint")}</div>
          </div>
        </div>
      ) : (
        <div className="card section" style={{ overflowX: "auto" }}>
          <table className="pool-matrix">
            <thead>
              <tr>
                <th>{t("pool.col.subscription")}</th>
                {CAPABILITIES.map((cap) => (
                  <th key={cap}>{CAPABILITY_LABEL[cap]}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((sub) => (
                <tr key={sub.id}>
                  <td className="pool-sub-cell">
                    <div className="pool-sub-name">{sub.display_name}</div>
                    <div className="text-xs text-muted-foreground">
                      <StatusBadge state={sub.state} /> {sub.enabled ? "" : t("pool.disabled")}
                    </div>
                  </td>
                  {CAPABILITIES.map((cap) => {
                    const entries = (bySub.get(sub.id) ?? []).filter((e) => e.capability === cap);
                    return (
                      <td key={cap}>
                        <button
                          type="button"
                          className={entries.length > 0 ? "pool-cell filled" : "pool-cell"}
                          onClick={() => open(sub.id)}
                          title={t("pool.cellHint")}
                        >
                          {entries.length === 0 ? (
                            <span className="text-muted-foreground">+</span>
                          ) : (
                            <span className="mono">
                              {entries
                                .map((e) => `${e.model_name}${e.enabled ? "" : " (off)"}`)
                                .join(" → ")}
                            </span>
                          )}
                        </button>
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <Dialog open={editing !== null} onOpenChange={(open) => !open && setEditing(null)}>
        <DialogContent style={{ maxWidth: 720 }}>
          <DialogHeader>
            <DialogTitle>{t("pool.editTitle")}</DialogTitle>
          </DialogHeader>
          {editing && (
            <ModelPoolPicker
              subscriptionId={editing}
              entries={draft}
              onChange={setDraft}
              models={editModels}
              loading={modelsLoading}
              onRefresh={() => void refreshEditorModels()}
            />
          )}
          <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
            <Button variant="outline" onClick={() => setEditing(null)}>
              {t("common.cancel")}
            </Button>
            <Button onClick={() => void save()} disabled={saving}>
              {t("common.save")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>

      {/* 订阅顺序: 全局统一 (5 个能力池共用), 多种按钮调整 */}
      <div className="page-header" style={{ marginTop: 24 }}>
        <h2>{t("pool.order.title")}</h2>
        <div className="subtitle">{t("pool.order.subtitle")}</div>
      </div>

      {orderRows.length === 0 ? (
        <div className="card section">
          <div className="card-body text-sm text-muted-foreground">{t("pool.order.empty")}</div>
        </div>
      ) : (
        <div className="card section">
          <div className="card-body" style={{ display: "grid", gap: 8 }}>
            <div className="field-hint" style={{ marginTop: 0 }}>{t("pool.order.hint")}</div>

            {/* 全局操作按钮: 打乱 / 反转 */}
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
              <button
                type="button"
                className="btn sm ghost"
                disabled={orderRows.length < 2 || reorderMut.isPending}
                onClick={shuffleOrder}
                title={t("pool.order.shuffle")}
              >
                <Shuffle size={13} style={{ marginRight: 4 }} />
                {t("pool.order.shuffle")}
              </button>
              <button
                type="button"
                className="btn sm ghost"
                disabled={orderRows.length < 2 || reorderMut.isPending}
                onClick={reverseOrder}
                title={t("pool.order.reverse")}
              >
                <ArrowUpDown size={13} style={{ marginRight: 4 }} />
                {t("pool.order.reverse")}
              </button>
            </div>

            {orderRows.map((sub, idx) => (
              <div
                key={sub.id}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 10,
                  padding: "8px 10px",
                  border: "1px solid var(--border-2)",
                  borderRadius: 8,
                }}
              >
                <span
                  className="priority mono"
                  style={{ width: 22, textAlign: "center", color: "var(--ink-2)", fontSize: 12 }}
                >
                  {idx + 1}
                </span>
                <ProviderLogo iconId={sub.provider_icon} size={18} iconSize={11} />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontWeight: 500 }}>{sub.display_name}</div>
                  <div className="text-xs text-muted-foreground">{capsOf(sub.id)}</div>
                </div>
                {/* 每行 4 个按钮: 置顶 / 上移 / 下移 / 置底 */}
                <div style={{ display: "flex", gap: 2 }}>
                  <button
                    type="button"
                    className="btn sm ghost"
                    style={{ padding: "4px 6px" }}
                    disabled={idx === 0 || reorderMut.isPending}
                    onClick={() => moveToTop(idx)}
                    title={t("pool.order.moveTop")}
                    aria-label={t("pool.order.moveTop")}
                  >
                    <ChevronsUp size={13} />
                  </button>
                  <button
                    type="button"
                    className="btn sm ghost"
                    style={{ padding: "4px 6px" }}
                    disabled={idx === 0 || reorderMut.isPending}
                    onClick={() => moveOrder(idx, -1)}
                    title={t("pool.order.moveUp")}
                    aria-label={t("pool.order.moveUp")}
                  >
                    <ChevronUp size={13} />
                  </button>
                  <button
                    type="button"
                    className="btn sm ghost"
                    style={{ padding: "4px 6px" }}
                    disabled={idx === orderRows.length - 1 || reorderMut.isPending}
                    onClick={() => moveOrder(idx, 1)}
                    title={t("pool.order.moveDown")}
                    aria-label={t("pool.order.moveDown")}
                  >
                    <ChevronDown size={13} />
                  </button>
                  <button
                    type="button"
                    className="btn sm ghost"
                    style={{ padding: "4px 6px" }}
                    disabled={idx === orderRows.length - 1 || reorderMut.isPending}
                    onClick={() => moveToBottom(idx)}
                    title={t("pool.order.moveBottom")}
                    aria-label={t("pool.order.moveBottom")}
                  >
                    <ChevronsDown size={13} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* 管理订阅: 能力池自己的订阅管理, 与经典「订阅管理」页分开 */}
      <div className="page-header" style={{ marginTop: 24 }}>
        <h2>{t("pool.manage.title")}</h2>
        <div className="subtitle">{t("pool.manage.subtitle")}</div>
      </div>

      {poolSubs.length === 0 ? (
        <div className="card section">
          <div className="card-body text-sm text-muted-foreground">{t("pool.manage.empty")}</div>
        </div>
      ) : (
        <div className="card section" style={{ overflowX: "auto" }}>
          <table className="pool-matrix">
            <thead>
              <tr>
                <th>{t("pool.col.subscription")}</th>
                <th>{t("pool.manage.caps")}</th>
                <th>{t("pool.manage.actions")}</th>
              </tr>
            </thead>
            <tbody>
              {poolSubs.map((sub) => {
                const entries = bySub.get(sub.id) ?? [];
                const activeCaps = CAPABILITIES.filter((c) =>
                  entries.some((e) => e.capability === c && e.enabled),
                );
                return (
                  <tr key={sub.id}>
                    <td className="pool-sub-cell">
                      <div className="pool-sub-name">{sub.display_name}</div>
                      <div className="text-xs text-muted-foreground">
                        <StatusBadge state={sub.state} /> {sub.enabled ? "" : t("pool.disabled")}
                      </div>
                    </td>
                    <td>
                      {activeCaps.length === 0 ? (
                        <span className="text-muted-foreground">—</span>
                      ) : (
                        activeCaps.map((c) => CAPABILITY_LABEL[c]).join(" / ")
                      )}
                    </td>
                    <td>
                      <div className="pool-manage-actions">
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => navigate(`/pool/subscriptions/${sub.id}`)}
                        >
                          {t("pool.manage.edit")}
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => navigate(`/pool/subscriptions/${sub.id}?test=1`)}
                        >
                          {t("pool.manage.test")}
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() =>
                            setEnabledMut.mutate({ id: sub.id, enabled: !sub.enabled })
                          }
                        >
                          {sub.enabled ? t("pool.manage.disable") : t("pool.manage.enable")}
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => setDeleting(sub)}
                        >
                          {t("pool.manage.delete")}
                        </Button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      <Dialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <DialogContent style={{ maxWidth: 420 }}>
          <DialogHeader>
            <DialogTitle>{t("pool.manage.deleteConfirmTitle")}</DialogTitle>
          </DialogHeader>
          <div className="text-sm text-muted-foreground">
            {t("pool.manage.deleteConfirmBody", { name: deleting?.display_name ?? "" })}
          </div>
          <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
            <Button variant="outline" onClick={() => setDeleting(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="destructive"
              disabled={deleteMut.isPending}
              onClick={() => {
                if (!deleting) return;
                deleteMut.mutate(deleting.id, { onSuccess: () => setDeleting(null) });
              }}
            >
              {t("pool.manage.delete")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
