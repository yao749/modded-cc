import { useMemo, useState } from "react";
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
import { useSubscriptions } from "@/hooks/useSubscriptions";
import { useT } from "@/i18n";
import { CAPABILITY_LABEL, type CapabilityCategory, type ModelPoolEntry } from "@/types";

const CAPABILITIES: CapabilityCategory[] = ["vision", "llm", "multimodal", "speech", "embedding"];

export const MODEL_POOLS_KEY = ["model-pools"] as const;

/**
 * 能力分类池总览: 行 = 订阅, 列 = 5 个能力。
 * 单元格显示该订阅在该能力下的优先级链 (按 priority 升序), 点击打开编辑器。
 * 池配置全部收敛到这里, 订阅编辑页不再出现池区块。
 */
export function PoolPage() {
  const { t } = useT();
  const subs = useSubscriptions();
  const queryClient = useQueryClient();
  const pools = useQuery({ queryKey: MODEL_POOLS_KEY, queryFn: () => api.listModelPools() });

  /** 正在编辑的订阅 */
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState<ModelPoolEntry[]>([]);
  const [saving, setSaving] = useState(false);

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

  function open(subId: string) {
    setDraft(bySub.get(subId) ?? []);
    setEditing(subId);
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

  return (
    <>
      <div className="page-header">
        <h1>{t("pool.title")}</h1>
        <div className="subtitle">{t("pool.subtitle")}</div>
      </div>

      {rows.length === 0 ? (
        <div className="card section">
          <div className="card-body text-sm text-muted-foreground">{t("pool.empty")}</div>
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
              models={null}
              loading={false}
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
    </>
  );
}
