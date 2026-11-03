import { useMemo, useState } from "react";
import { useNavigate } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { ArrowRight, Copy } from "lucide-react";
import { api } from "@/api/tauri";
import { useProxyStatus } from "@/hooks/useSettings";
import { useUpdateVirtualModel, useVirtualModels } from "@/hooks/useVirtualModels";
import { VM_META } from "@/lib/virtualModels";
import { runtime } from "@/runtime";
import { useT } from "@/i18n";
import {
  CAPABILITY_VIRTUAL_MODEL,
  type CapabilityCategory,
  type ModelPoolEntry,
  type RoutingMode,
  type VirtualModelDto,
  type VirtualModelName,
} from "@/types";

/** 更新指定能力虚拟模型的调度模式 —— 与经典四槽卡片 update 签名一致。 */
function setVmMode(
  updateMut: ReturnType<typeof useUpdateVirtualModel>,
  vmName: VirtualModelName,
  vm: VirtualModelDto,
  mode: RoutingMode,
) {
  updateMut.mutate({ name: vmName, input: { mode, subscription_ids: vm.subscription_ids } });
}

/** 与 /pool 总览页共用同一个 query key, 两页共享缓存。 */
const MODEL_POOLS_KEY = ["model-pools"] as const;

/** 行的展示顺序: 主对话在最前 (最常查), 其余按模态从大到小。 */
const CAP_ORDER: CapabilityCategory[] = ["llm", "vision", "multimodal", "speech", "embedding"];

interface ChainRow {
  capability: CapabilityCategory;
  vmName: VirtualModelName;
  subs: number;
  models: number;
  /** 链头模型 (priority 最小且启用), 空链为 null */
  head: string | null;
  /** 该能力对应的虚拟模型条目 (含 mode + subscription_ids), 未加载到为 null */
  vm: VirtualModelDto | null;
}

/**
 * 「池」范式块 —— 与四槽 / model-jev 并列的第 7 块, 放在 jev 决策卡上方。
 *
 * 原理与四槽一致: 一个虚拟模型名, 一张绑定表, 一份调度模式; 区别是池的链路粒度
 * 落在「模型」而不是「订阅」——同一条链里可以有多条订阅的多个模型, 按 priority
 * 排队, 失败的模型被挪到队尾。所以这里只做总览 (每链几订阅 / 几模型 / 队头是谁),
 * 增删改仍在「能力池」页 (模型池的编辑要拖 20 个模型, 塞进这张卡会毁掉页面)。
 */
export function PoolParadigmCard() {
  const { t } = useT();
  const navigate = useNavigate();
  const updateMut = useUpdateVirtualModel();
  const pools = useQuery({ queryKey: MODEL_POOLS_KEY, queryFn: () => api.listModelPools() });
  const vms = useVirtualModels();

  const rows = useMemo<ChainRow[]>(() => {
    const entries: ModelPoolEntry[] = pools.data ?? [];
    return CAP_ORDER.map((capability) => {
      const vmName = CAPABILITY_VIRTUAL_MODEL[capability] as VirtualModelName;
      const chain = entries
        .filter((e) => e.capability === capability && e.enabled)
        .sort((a, b) => a.priority - b.priority);
      const vm = vms.data?.find((v) => v.name === vmName) ?? null;
      return {
        capability,
        vmName,
        subs: new Set(chain.map((e) => e.subscription_id)).size,
        models: chain.length,
        head: chain[0]?.model_name ?? null,
        vm,
      };
    });
  }, [pools.data, vms.data]);

  const configured = rows.filter((r) => r.models > 0).length;

  return (
    <div className="slot-card wide pool">
      <div className="jev-hangtag pool-hangtag" aria-hidden="true">
        <span className="jev-hangtag-string" />
        <span className="jev-hangtag-tag pool-tag">
          POOL
          <small>{t("virtualModels.pool.tagSub")}</small>
        </span>
      </div>

      <div className="slot-head compact">
        <div style={{ minWidth: 0 }}>
          <span className="slot-name">{t("virtualModels.pool.title")}</span>
          <span className="slot-purpose">
            <strong>{t("vm.pool.purpose")}</strong> · {t("vm.pool.purposeEn")}
          </span>
        </div>
        <button className="btn pool-manage" type="button" onClick={() => navigate("/pool")}>
          {t("virtualModels.pool.manage")} <ArrowRight size={12} />
        </button>
      </div>

      <div className="slot-body pool-body">
        <div className="pool-chains">
          {rows.map((row) => {
            const meta = VM_META[row.vmName];
            const empty = row.models === 0;
            return (
              <div
                key={row.vmName}
                className={empty ? "pool-chain empty" : "pool-chain"}
                onClick={() => navigate("/pool")}
                role="button"
                tabIndex={0}
                aria-label={`${row.vmName} ${t(meta.purposeKey)}`}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    navigate("/pool");
                  }
                }}
                title={t("virtualModels.pool.rowHint")}
              >
                <span className="pool-chain-name mono">{row.vmName}</span>
                <span className="pool-chain-label">
                  {t(meta.purposeKey)} · {t(meta.purposeEnKey)}
                </span>
                <span className="pool-chain-stats">
                  {empty
                    ? t("virtualModels.pool.notConfigured")
                    : t("virtualModels.pool.stats", { subs: row.subs, models: row.models })}
                </span>
                <span
                  className={empty ? "pool-chain-head empty mono" : "pool-chain-head mono"}
                  title={row.head ?? ""}
                >
                  {row.head ?? "—"}
                </span>
                {/* 调度模式: 与经典四槽卡片同款三按钮 radio-group */}
                {row.vm && (
                  <div
                    className="radio-group sm pool-chain-mode-group"
                    role="group"
                    aria-label={t("virtualModels.mode.label")}
                    onClick={(e) => e.stopPropagation()}
                    onKeyDown={(e) => {
                      // 只拦截会触发外层 button 导航的按键, Tab 等正常冒泡
                      if (e.key === "Enter" || e.key === " ") e.stopPropagation();
                    }}
                  >
                    <button
                      type="button"
                      className={row.vm.mode === "sequential" ? "on" : ""}
                      onClick={() => setVmMode(updateMut, row.vmName, row.vm!, "sequential")}
                    >
                      {t("vm.mode.sequential")}
                    </button>
                    <button
                      type="button"
                      className={row.vm.mode === "round_robin" ? "on" : ""}
                      onClick={() => setVmMode(updateMut, row.vmName, row.vm!, "round_robin")}
                    >
                      {t("vm.mode.round_robin")}
                    </button>
                    <button
                      type="button"
                      className={row.vm.mode === "sticky" ? "on" : ""}
                      onClick={() => setVmMode(updateMut, row.vmName, row.vm!, "sticky")}
                    >
                      {t("vm.mode.sticky")}
                    </button>
                  </div>
                )}
              </div>
            );
          })}
        </div>
        <div className="pool-side">
          <div className="pool-side-count">
            <b>{configured}</b>
            <span>{t("virtualModels.pool.configured", { total: rows.length })}</span>
          </div>
          <PoolAccessHint />
        </div>
      </div>
    </div>
  );
}

/**
 * 接入片段: 池的调用方写能力名, 不写真实模型名。
 * 这是最容易踩的坑 —— 上游模型的真名 (glm-5.3 之类) 不属于任何虚拟名, 会被当成
 * 「未知模型」丢给 model-fallback (四槽的兜底), 在只有池订阅的机器上直接 503。
 */
function PoolAccessHint() {
  const { t } = useT();
  const status = useProxyStatus();
  const [copied, setCopied] = useState(false);
  const base = status.data?.base_url ?? "http://127.0.0.1:23456";
  const curl = [
    `curl ${base}/v1/chat/completions \\`,
    `  -H "Content-Type: application/json" \\`,
    `  -H "Authorization: Bearer <cc-router token>" \\`,
    `  -d '{"model":"model-llm","messages":[{"role":"user","content":"hi"}],"max_tokens":16}'`,
  ].join("\n");
  return (
    <div className="jev-access pool-access">
      <b>{t("virtualModels.pool.access")}</b>
      <span className="mono">{base}/v1/chat/completions</span>
      <span>{t("virtualModels.pool.modelHint")}</span>
      <button
        type="button"
        className="jev-copy"
        onClick={() => {
          runtime
            .copyText(curl)
            .then(() => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            })
            .catch(() => {});
        }}
      >
        <Copy size={11} /> {copied ? t("virtualModels.pool.copied") : t("virtualModels.pool.copyCurl")}
      </button>
    </div>
  );
}
