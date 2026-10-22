import { useMemo, useState } from "react";
import { RefreshCw, CircleAlert, ChevronUp, ChevronDown, Trash2, Plus, ToggleLeft, ToggleRight, Check, X, Minus, CircleHelp } from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  CAPABILITY_LABEL,
  defaultInputsForCapability,
  type CapabilityCategory,
  type InputModality,
  type ModelInfo,
  type ModelPoolEntry,
  type PoolTestItem,
} from "@/types";
import { api } from "@/api/tauri";

const CAPABILITIES: CapabilityCategory[] = ["vision", "llm", "multimodal", "speech", "embedding"];

interface Props {
  /** 当前订阅 id (load/save 时用) */
  subscriptionId: string;
  /** 当前已有池条目 (编辑态从后端加载) */
  entries: ModelPoolEntry[];
  onChange: (next: ModelPoolEntry[]) => void;
  /** 该订阅可用的模型列表 (从 refresh_model_list 拿的全量) */
  models: ModelInfo[] | null;
  loading?: boolean;
  onRefresh?: () => void;
  disabled?: boolean;
}

/** 5 个能力分类 Tab + 每类下多个模型行 (可排序/可切换启用/可删除). */
export function ModelPoolPicker({
  subscriptionId,
  entries,
  onChange,
  models,
  loading,
  onRefresh,
  disabled,
}: Props) {
  const [active, setActive] = useState<CapabilityCategory>("llm");
  const [testLoading, setTestLoading] = useState(false);
  /** 按 model_name 索引的测试结果, 用于每行前面显示 ✓/✗/- 状态 */
  const [testResults, setTestResults] = useState<Record<string, PoolTestItem>>({});

  // 切换 tab 时清空上次测试结果
  function switchCap(c: CapabilityCategory) {
    setActive(c);
    setTestResults({});
  }

  async function runPoolTest() {
    setTestLoading(true);
    setTestResults({});
    try {
      const list = await api.testPoolConnection(subscriptionId, active);
      const idx: Record<string, PoolTestItem> = {};
      for (const item of list) {
        idx[item.model_name] = item;
      }
      setTestResults(idx);
    } catch (e) {
      // 整批失败, 给一个占位
      setTestResults({
        "(error)": {
          model_name: "(error)",
          priority: 0,
          enabled: false,
          ok: false,
          message: String(e),
        },
      });
    } finally {
      setTestLoading(false);
    }
  }

  // 同一能力分类下按 priority 排序
  const entriesByCap = useMemo(() => {
    const out: Record<CapabilityCategory, ModelPoolEntry[]> = {
      vision: [], llm: [], multimodal: [], speech: [], embedding: [],
    };
    for (const e of entries) {
      (out[e.capability] ??= []).push(e);
    }
    for (const cap of CAPABILITIES) {
      out[cap].sort((a, b) => a.priority - b.priority);
    }
    return out;
  }, [entries]);

  function updateCap(cap: CapabilityCategory, idx: number, next: Partial<ModelPoolEntry>) {
    const list = entriesByCap[cap];
    const target = list[idx];
    // 先把 target 从全量 entries 里去掉, 再把更新后的塞回去
    const others = entries.filter((e) => e.id !== target.id);
    onChange([...others, { ...target, ...next }]);
  }

  function remove(cap: CapabilityCategory, idx: number) {
    const list = entriesByCap[cap];
    onChange(entries.filter((e) => e.id !== list[idx].id));
  }

  function moveUp(cap: CapabilityCategory, idx: number) {
    if (idx === 0) return;
    const list = entriesByCap[cap];
    const a = list[idx - 1];
    const b = list[idx];
    const others = entries.filter((e) => e.id !== a.id && e.id !== b.id);
    onChange([
      ...others,
      { ...a, priority: b.priority },
      { ...b, priority: a.priority },
    ]);
  }

  function moveDown(cap: CapabilityCategory, idx: number) {
    const list = entriesByCap[cap];
    if (idx >= list.length - 1) return;
    const a = list[idx];
    const b = list[idx + 1];
    const others = entries.filter((e) => e.id !== a.id && e.id !== b.id);
    onChange([
      ...others,
      { ...a, priority: b.priority },
      { ...b, priority: a.priority },
    ]);
  }

  /** 从模型发现列表里选一个, 或手动输入, 添加到当前 active 能力分类池末位 */
  function addToCurrent(modelName?: string) {
    // 最大 priority + 1 当末位
    const existing = entriesByCap[active];
    const maxPri = existing.reduce((m, e) => Math.max(m, e.priority), -1);
    const newEntry: ModelPoolEntry = {
      id: `pool-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      subscription_id: subscriptionId,
      capability: active,
      model_name: modelName ?? "",
      priority: maxPri + 1,
      enabled: true,
      supported_inputs: defaultInputsForCapability(active),
    };
    onChange([...entries, newEntry]);
  }

  /** 一键把所有 inferred 匹配当前 active 能力分类的模型加入池末位 (已加过的跳过) */
  function autoFillFromDiscovery() {
    if (!models) return;
    const existingNames = new Set(entriesByCap[active].map((e) => e.model_name));
    const candidates = models.filter(
      (m) => m.inferred_capability === active && !existingNames.has(m.id),
    );
    if (candidates.length === 0) return;
    const existing = entriesByCap[active];
    let nextPri = existing.reduce((m, e) => Math.max(m, e.priority), -1) + 1;
    const newEntries: ModelPoolEntry[] = candidates.map((m) => ({
      id: `pool-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      subscription_id: subscriptionId,
      capability: active,
      model_name: m.id,
      priority: nextPri++,
      enabled: true,
      supported_inputs: defaultInputsForCapability(active),
    }));
    onChange([...entries, ...newEntries]);
  }

  // 当前模型发现列表里 inferred == active 的候选
  const autoFillCandidates = useMemo(() => {
    if (!models) return [] as ModelInfo[];
    const existingNames = new Set(entriesByCap[active].map((e) => e.model_name));
    return models.filter(
      (m) => m.inferred_capability === active && !existingNames.has(m.id),
    );
  }, [models, entriesByCap, active]);

  return (
    <div>
      {/* 顶部: 分类 Tab + 刷新按钮 */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: 12,
          gap: 12,
          flexWrap: "wrap",
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 4,
            flexWrap: "wrap",
          }}
        >
          {CAPABILITIES.map((cap) => {
            const count = entriesByCap[cap].length;
            const isActive = cap === active;
            return (
              <button
                key={cap}
                type="button"
                onClick={() => switchCap(cap)}
                disabled={disabled}
                className={isActive ? "cap-tab on" : "cap-tab"}
              >
                {CAPABILITY_LABEL[cap]}
                {count > 0 && (
                  <span className="cap-badge">{count}</span>
                )}
              </button>
            );
          })}
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <button
            type="button"
            className="btn sm"
            onClick={runPoolTest}
            disabled={disabled || testLoading}
            title={`逐个测试 ${CAPABILITY_LABEL[active]} 分类下所有模型的连通性`}
          >
            <RefreshCw size={12} className={testLoading ? "animate-spin" : undefined} />
            {testLoading ? "测试中..." : `全部测试 ${CAPABILITY_LABEL[active]}`}
          </button>
          {autoFillCandidates.length > 0 && (
            <button
              type="button"
              className="btn sm"
              onClick={autoFillFromDiscovery}
              disabled={disabled}
              title={`按自动推断加入 ${autoFillCandidates.length} 个候选模型`}
            >
              <Plus size={12} />
              自动填充 ({autoFillCandidates.length})
            </button>
          )}
          {onRefresh && (
            <button
              className="btn sm"
              onClick={onRefresh}
              disabled={disabled || loading}
              type="button"
            >
              <RefreshCw size={12} className={loading ? "animate-spin" : undefined} />
              刷新模型列表
            </button>
          )}
        </div>
      </div>

      {/* 说明 */}
      <div className="field-hint" style={{ marginBottom: 10, marginTop: 0 }}>
        {CAPABILITY_LABEL[active]} 能力分类下的模型, 将以 <code>model-{active === "multimodal" ? "multimodal" : active}</code> 名称对外暴露.
        路由时按优先级 (数字最小优先) 取可用模型; 首选失败自动降级到下一个.
      </div>

      {/* 测试结果汇总条 — 批量测试后一行显示成功/失败/跳过统计 */}
      {Object.keys(testResults).length > 0 && !testResults["(error)"] && (
        <div
          style={{
            marginBottom: 10,
            padding: "8px 12px",
            borderRadius: 6,
            border: "1px solid var(--border-2)",
            background: "var(--bg-elev, transparent)",
            fontSize: 12,
            display: "flex",
            gap: 14,
            alignItems: "center",
          }}
        >
          {(() => {
            const items = Object.values(testResults);
            const ok = items.filter((t) => t.enabled && t.ok).length;
            const fail = items.filter((t) => t.enabled && !t.ok).length;
            const skip = items.filter((t) => !t.enabled).length;
            return (
              <>
                <span style={{ fontWeight: 600 }}>测试结果 ({items.length})</span>
                {ok > 0 && (
                  <span style={{ color: "#22c55e" }}>
                    <Check size={11} style={{ verticalAlign: -2, marginRight: 2 }} />
                    成功 {ok}
                  </span>
                )}
                {fail > 0 && (
                  <span style={{ color: "#ef4444" }}>
                    <X size={11} style={{ verticalAlign: -2, marginRight: 2 }} />
                    失败 {fail}
                  </span>
                )}
                {skip > 0 && (
                  <span style={{ color: "#9ca3af" }}>
                    <Minus size={11} style={{ verticalAlign: -2, marginRight: 2 }} />
                    跳过 {skip}
                  </span>
                )}
              </>
            );
          })()}
        </div>
      )}

      {/* 列头 */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          marginBottom: 8,
          paddingRight: 104, // 给右侧按钮组留空间保持对齐
        }}
      >
        <div style={{ flex: 1, minWidth: 0, fontSize: 12, fontWeight: 500, color: "var(--ink-3)" }}>
          模型名称
        </div>
        <div style={{ width: 72, fontSize: 12, fontWeight: 500, color: "var(--ink-3)", textAlign: "right" }}>
          启用
        </div>
        <div style={{ width: 104 }} />
      </div>

      {/* 行列表 */}
      <div style={{ display: "grid", gap: 10 }}>
        {entriesByCap[active].length === 0 && (
          <div
            style={{
              padding: "20px 12px",
              textAlign: "center",
              color: "var(--ink-3)",
              fontSize: 13,
              border: "1px dashed var(--border-2)",
              borderRadius: 8,
            }}
          >
            <div style={{ marginBottom: 8 }}>该分类下还没有模型</div>
            <div style={{ display: "flex", gap: 8, justifyContent: "center" }}>
              {autoFillCandidates.length > 0 && (
                <button type="button" className="btn sm" onClick={autoFillFromDiscovery} disabled={disabled}>
                  <Plus size={12} /> 从自动推断添加
                </button>
              )}
              <button
                type="button"
                className="btn sm ghost"
                onClick={() => addToCurrent()}
                disabled={disabled}
              >
                <Plus size={12} /> 手动输入
              </button>
            </div>
          </div>
        )}

        {entriesByCap[active].map((entry, idx) => (
          <EntryRow
            key={entry.id}
            entry={entry}
            idx={idx}
            total={entriesByCap[active].length}
            models={models}
            disabled={disabled}
            testItem={testResults[entry.model_name]}
            onModelNameChange={(v) => updateCap(active, idx, { model_name: v })}
            onInputsChange={(v) => updateCap(active, idx, { supported_inputs: v })}
            onEnabledToggle={() => updateCap(active, idx, { enabled: !entry.enabled })}
            onMoveUp={() => moveUp(active, idx)}
            onMoveDown={() => moveDown(active, idx)}
            onRemove={() => remove(active, idx)}
          />
        ))}
      </div>

      {/* 底部加一行 */}
      {entriesByCap[active].length > 0 && (
        <div style={{ marginTop: 12 }}>
          <button
            type="button"
            className="btn sm ghost"
            onClick={() => addToCurrent()}
            disabled={disabled}
          >
            <Plus size={12} /> 添加模型
          </button>
        </div>
      )}
    </div>
  );
}

/** 单行: 模型输入 + 模态声明 + 启用开关 + 上/下/删 */
function EntryRow({
  entry,
  idx,
  total,
  models,
  disabled,
  testItem,
  onModelNameChange,
  onInputsChange,
  onEnabledToggle,
  onMoveUp,
  onMoveDown,
  onRemove,
}: {
  entry: ModelPoolEntry;
  idx: number;
  total: number;
  models: ModelInfo[] | null;
  disabled?: boolean;
  /** 该模型的连通性测试结果 (undefined = 未测过) */
  testItem?: PoolTestItem;
  onModelNameChange: (v: string) => void;
  onInputsChange: (v: InputModality[]) => void;
  onEnabledToggle: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
  onRemove: () => void;
}) {
  const discoveryCandidates = useMemo(() => {
    if (!models) return [];
    return models
      .filter((m) => !m.id.startsWith("@cf/cloudflare/clef"))
      .sort((a, b) => a.id.localeCompare(b.id));
  }, [models]);

  const inferredMismatch = useMemo(() => {
    if (!models) return null;
    const found = models.find((m) => m.id === entry.model_name);
    if (found?.inferred_capability && found.inferred_capability !== entry.capability) {
      return found.inferred_capability;
    }
    return null;
  }, [models, entry.model_name, entry.capability]);

  /** 切换某个模态的选中状态, 至少保留 text */
  function toggleInput(mod: InputModality) {
    const set = new Set(entry.supported_inputs);
    if (set.has(mod)) {
      set.delete(mod);
      // 至少保留 text
      if (!set.has("text") && mod !== "text") {
        set.add("text");
      }
    } else {
      set.add(mod);
    }
    onInputsChange(Array.from(set));
  }

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "8px 10px",
        borderRadius: 8,
        border: "1px solid var(--border-2)",
        background: "var(--bg-elev, transparent)",
        opacity: entry.enabled ? 1 : 0.55,
      }}
    >
      {/* 连通性状态图标: 未测试→虚线圈问号 / 成功→绿圆白勾 / 失败→红圆白叉 / 未启用→灰圆白减号 */}
      {!testItem ? (
        <div
          style={{
            width: 22,
            height: 22,
            borderRadius: 11,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            flexShrink: 0,
            border: "1.5px dashed #9ca3af",
            background: "transparent",
          }}
          title="未测试 — 点击右上角「全部测试」按钮批量探测"
        >
          <CircleHelp size={13} color="#9ca3af" />
        </div>
      ) : !testItem.enabled ? (
        <div
          style={{
            width: 22,
            height: 22,
            borderRadius: 11,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            flexShrink: 0,
            background: "#9ca3af",
          }}
          title="未启用, 跳过测试"
        >
          <Minus size={14} color="#ffffff" strokeWidth={3} />
        </div>
      ) : testItem.ok ? (
        <div
          style={{
            width: 22,
            height: 22,
            borderRadius: 11,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            flexShrink: 0,
            background: "#22c55e",
          }}
          title={`连通成功${testItem.http_status ? ` · HTTP ${testItem.http_status}` : ""}`}
        >
          <Check size={14} color="#ffffff" strokeWidth={3} />
        </div>
      ) : (
        <div
          style={{
            width: 22,
            height: 22,
            borderRadius: 11,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            flexShrink: 0,
            background: "#ef4444",
          }}
          title={`连通失败 · ${testItem.message}${testItem.http_status ? ` · HTTP ${testItem.http_status}` : ""}`}
        >
          <X size={14} color="#ffffff" strokeWidth={3} />
        </div>
      )}
      {/* 序号 + 优先级 badge */}
      <div
        style={{
          width: 22,
          height: 22,
          borderRadius: 11,
          background: "var(--bg-3, #1f2937)",
          color: "var(--ink-2)",
          fontSize: 11,
          fontWeight: 600,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          flexShrink: 0,
        }}
        title={`优先级 ${entry.priority}`}
      >
        {idx + 1}
      </div>

      {/* 模型输入 (优先用下拉, 没有发现列表时退化成输入) */}
      <div style={{ flex: 1, minWidth: 0 }}>
        {discoveryCandidates.length > 0 ? (
          <Select
            value={entry.model_name}
            onValueChange={onModelNameChange}
            disabled={disabled}
          >
            <SelectTrigger className="font-mono" style={{ width: "100%", height: 30 }}>
              <SelectValue placeholder="选择或输入模型名" />
            </SelectTrigger>
            <SelectContent>
              {/* 总是把当前值放进来避免 Radix 卸载选中项 */}
              <SelectItem value={entry.model_name} className="font-mono">
                {entry.model_name}
              </SelectItem>
              {discoveryCandidates
                .filter((m) => m.id !== entry.model_name)
                .map((m) => (
                  <SelectItem key={m.id} value={m.id} className="font-mono">
                    {m.display_name || m.id}
                    {m.inferred_capability && (
                      <span style={{ marginLeft: 6, opacity: 0.6, fontSize: 11 }}>
                        · 推断: {CAPABILITY_LABEL[m.inferred_capability]}
                      </span>
                    )}
                  </SelectItem>
                ))}
            </SelectContent>
          </Select>
        ) : (
          <input
            className="input mono"
            style={{ width: "100%", height: 30 }}
            value={entry.model_name}
            onChange={(e) => onModelNameChange(e.target.value)}
            placeholder="输入上游模型名 (如 gpt-4o)"
            disabled={disabled}
          />
        )}
        {inferredMismatch && (
          <div
            style={{
              marginTop: 4,
              display: "flex",
              alignItems: "center",
              gap: 4,
              color: "var(--warn, #d97706)",
              fontSize: 11,
            }}
          >
            <CircleAlert size={11} />
            模型名匹配到, 但自动推断分类是「{CAPABILITY_LABEL[inferredMismatch]}」
          </div>
        )}
      </div>

      {/* 模态声明 —— 借鉴 DSH 的 input: [text, image] */}
      <div
        style={{
          display: "flex",
          gap: 4,
          flexShrink: 0,
        }}
        title="该模型支持的输入模态, 路由时会校验请求体是否兼容"
      >
        {(["text", "image", "audio"] as InputModality[]).map((mod) => {
          const active = entry.supported_inputs.includes(mod);
          return (
            <button
              key={mod}
              type="button"
              onClick={() => toggleInput(mod)}
              disabled={disabled || (mod === "text" && entry.supported_inputs.length <= 1 && active)}
              style={{
                padding: "2px 7px",
                fontSize: 11,
                borderRadius: 10,
                border: active
                  ? "1.5px solid var(--accent, #3b82f6)"
                  : "1.5px solid var(--border-2)",
                background: active ? "var(--fill-butter, #fff5d6)" : "transparent",
                color: active ? "var(--ink)" : "var(--ink-3)",
                fontWeight: active ? 600 : 400,
                cursor: disabled ? "not-allowed" : "pointer",
                whiteSpace: "nowrap",
              }}
            >
              {mod === "text" ? "文本" : mod === "image" ? "图片" : "音频"}
            </button>
          );
        })}
      </div>

      {/* 启用开关 */}
      <button
        type="button"
        onClick={onEnabledToggle}
        disabled={disabled}
        title={entry.enabled ? "点击禁用" : "点击启用"}
        style={{
          width: 36,
          height: 22,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          color: entry.enabled ? "var(--accent, #3b82f6)" : "var(--ink-3)",
          flexShrink: 0,
          cursor: disabled ? "not-allowed" : "pointer",
          border: "none",
          background: "transparent",
          padding: 0,
        }}
      >
        {entry.enabled ? <ToggleRight size={22} /> : <ToggleLeft size={22} />}
      </button>

      {/* 按钮组: 上/下/删 */}
      <div
        style={{
          display: "flex",
          gap: 2,
          flexShrink: 0,
        }}
      >
        <button
          type="button"
          className="btn sm ghost"
          onClick={onMoveUp}
          disabled={disabled || idx === 0}
          title="上移 (优先级降低 → 下一个)"
          style={{ padding: "4px 6px" }}
        >
          <ChevronUp size={13} />
        </button>
        <button
          type="button"
          className="btn sm ghost"
          onClick={onMoveDown}
          disabled={disabled || idx >= total - 1}
          title="下移"
          style={{ padding: "4px 6px" }}
        >
          <ChevronDown size={13} />
        </button>
        <button
          type="button"
          className="btn sm ghost"
          onClick={onRemove}
          disabled={disabled}
          title="删除"
          style={{ padding: "4px 6px", color: "var(--danger, #dc2626)" }}
        >
          <Trash2 size={13} />
        </button>
      </div>
    </div>
  );
}
