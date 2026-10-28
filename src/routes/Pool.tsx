import { useEffect, useMemo, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
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
