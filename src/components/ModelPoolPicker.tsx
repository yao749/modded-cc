import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { RefreshCw, CircleAlert, ChevronUp, ChevronDown, Trash2, Plus, ToggleLeft, ToggleRight, Check, X, Minus, CircleHelp, Sparkles } from "lucide-react";
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

/** 测试结果的行键: 同一模型可能同时挂在两条能力链里, 只用模型名会串色 */
export const poolTestKey = (cap: string, model: string) => `${cap}\u0000${model}`;

interface Props {
  /** 当前订阅 id (load/save 时用) */
  subscriptionId: string;
  /** 当前已有池条目 (编辑态从后端加载) */
  entries: ModelPoolEntry[];
  onChange: (next: ModelPoolEntry[]) => void;
  /** 该订阅可用的模型列表 (从 refresh_model_list 拿的全量) */
  models: ModelInfo[] | null;
  loading?: boolean;
  /** 刷新模型列表; 返回拉取到的模型, 供「自动获取并填充」在同一次点击内直接使用 */
  onRefresh?: () => void | Promise<ModelInfo[] | null>;
  disabled?: boolean;
  /**
   * 由父组件托管测试结果: 传入后本组件不再自己跑测试, 颜色/进度/统计全部读这里。
   * 页面级「按列表测试全部模型」和组件内「全部测试 <tab>」由此共用同一份结果 ——
   * 否则两套状态各记一份, 用户点不同按钮会看到"颜色时有时无"。
   */
  externalTest?: {
    /** 键 = poolTestKey(capability, model) */
    results: Record<string, PoolTestItem>;
    /** 正在测的那一行 */
    testingKey: string | null;
    running: boolean;
    progress: { done: number; total: number } | null;
    /** 触发某能力分类的逐行测试 */
    run: (cap: CapabilityCategory) => void;
  };
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
  externalTest,
}: Props) {
  const [active, setActive] = useState<CapabilityCategory>("llm");
  const [testLoading, setTestLoading] = useState(false);
  /** 测试结果: 键 = `能力分类\0模型名` —— 同一个模型可能同时挂在两条链里, 只用模型名会串色 */
  const [testResults, setTestResults] = useState<Record<string, PoolTestItem>>({});
  /** 正在测的那一行 (逐行测试 → 该行单独转圈) */
  const [testingKey, setTestingKey] = useState<string | null>(null);
  /** 逐行测试进度 (仅当前 tab) */
  const [testProgress, setTestProgress] = useState<{ done: number; total: number } | null>(null);
  /** 「自动获取并填充」的执行态 + 结果提示 */
  const [autoFillAllLoading, setAutoFillAllLoading] = useState(false);
  const [autoFillAllMsg, setAutoFillAllMsg] = useState<string | null>(null);

  // 结果来源: 父组件托管时一律读 externalTest, 否则用本地自己跑出来的那份
  const results = externalTest ? externalTest.results : testResults;
  const activeTestingKey = externalTest ? externalTest.testingKey : testingKey;
  const testRunning = externalTest ? externalTest.running : testLoading;
  const testProg = externalTest ? externalTest.progress : testProgress;

  // 切换 tab 不再清空结果: 各分类分开存, 切回来颜色还在 (之前清空是"颜色时有时无"的原因)
  function switchCap(c: CapabilityCategory) {
    setActive(c);
  }

  async function runPoolTest() {
    if (testLoading) return;
    const cap = active;
    setTestLoading(true);
    setTestingKey(null);
    try {
      // 以库里的条目为准: 刚加还没保存的行发给后端只会报"不在链里", 先标灰并说明
      const saved = await api.loadModelPool(subscriptionId).catch(() => [] as ModelPoolEntry[]);
      const savedModels = new Set(
        saved.filter((e) => e.capability === cap).map((e) => e.model_name),
      );
      const rows = entriesByCap[cap];

      // 先铺一遍状态, 整列立刻有反馈: 未保存/未启用灰掉, 待测的留空转圈
      const seed: Record<string, PoolTestItem> = { ...testResults };
      for (const r of rows) {
        const key = poolTestKey(cap, r.model_name);
        if (!savedModels.has(r.model_name)) {
          seed[key] = {
            model_name: r.model_name,
            priority: r.priority,
            enabled: false,
            ok: false,
            message: "尚未保存到订阅, 保存后才能测试",
          };
        } else if (!r.enabled) {
          seed[key] = {
            model_name: r.model_name,
            priority: r.priority,
            enabled: false,
            ok: false,
            message: "未启用, 跳过测试",
          };
        } else {
          delete seed[key];
        }
      }
      setTestResults(seed);

      // 逐行测试: 每测完一个就写一个结果, 颜色立刻出现, 不等整条链跑完
      const queue = rows.filter((r) => r.enabled && savedModels.has(r.model_name));
      setTestProgress({ done: 0, total: queue.length });
      for (let i = 0; i < queue.length; i++) {
        const r = queue[i];
        const key = poolTestKey(cap, r.model_name);
        setTestingKey(key);
        try {
          const item = await api.testPoolModel(subscriptionId, cap, r.model_name);
          setTestResults((prev) => ({ ...prev, [key]: item }));
        } catch (e) {
          setTestResults((prev) => ({
            ...prev,
            [key]: {
              model_name: r.model_name,
              priority: r.priority,
              enabled: true,
              ok: false,
              message: String(e),
            },
          }));
        } finally {
          setTestProgress({ done: i + 1, total: queue.length });
        }
      }
    } finally {
      setTestingKey(null);
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

  /** 把模型发现列表里所有能推断出能力分类的模型, 一次性填入对应分类 (已有跳过). 返回新增条数. */
  function fillAllFromDiscovery(source: ModelInfo[]): number {
    const existing = new Set(entries.map((e) => `${e.capability}\u0000${e.model_name}`));
    // 每个分类的起始 priority = 该类现有最大 priority + 1
    const nextPri: Record<string, number> = {};
    for (const cap of CAPABILITIES) {
      nextPri[cap] = entries
        .filter((e) => e.capability === cap)
        .reduce((m, e) => Math.max(m, e.priority + 1), 0);
    }
    const added: ModelPoolEntry[] = [];
    for (const m of source) {
      const cap = m.inferred_capability as CapabilityCategory | null | undefined;
      if (!cap || !(cap in nextPri)) continue;
      const key = `${cap}\u0000${m.id}`;
      if (existing.has(key)) continue;
      existing.add(key);
      added.push({
        id: `pool-${Date.now()}-${Math.random().toString(36).slice(2, 7)}-${added.length}`,
        subscription_id: subscriptionId,
        capability: cap,
        model_name: m.id,
        priority: nextPri[cap]++,
        enabled: true,
        supported_inputs: defaultInputsForCapability(cap),
      });
    }
    if (added.length > 0) onChange([...entries, ...added]);
    return added.length;
  }

  /**
   * 一键: 模型列表为空时先自动获取 (onRefresh), 再按推断把 5 个能力分类一次性填好。
   * 这是池页面「自动获取模型」的主入口 —— 不用切 5 次 tab 逐个点自动填充。
   */
  async function autoFetchAndFillAll() {
    setAutoFillAllLoading(true);
    setAutoFillAllMsg(null);
    try {
      let list: ModelInfo[] | null = models;
      if (!list || list.length === 0) {
        const fetched = await onRefresh?.();
        if (fetched && fetched.length > 0) list = fetched;
      }
      if (!list || list.length === 0) {
        setAutoFillAllMsg("未获取到模型列表: 请确认 API Key / Base URL 无误, 或点「刷新模型列表」重试");
        return;
      }
      const added = fillAllFromDiscovery(list);
      setAutoFillAllMsg(
        added > 0
          ? `已自动填入 ${added} 个模型 (按推断分到各能力分类)`
          : "可推断的模型都已加入, 没有新增",
      );
    } finally {
      setAutoFillAllLoading(false);
    }
  }

  // 当前模型发现列表里 inferred == active 的候选
  const autoFillCandidates = useMemo(() => {
    if (!models) return [] as ModelInfo[];
    const existingNames = new Set(entriesByCap[active].map((e) => e.model_name));
    return models.filter(
      (m) => m.inferred_capability === active && !existingNames.has(m.id),
    );
  }, [models, entriesByCap, active]);

  // 5 个分类合计的可自动填充候选 (按钮上显示的数字)
  const totalAutoFillCandidates = useMemo(() => {
    if (!models) return 0;
    const existing = new Set(entries.map((e) => `${e.capability}\u0000${e.model_name}`));
    return models.filter((m) => {
      const cap = m.inferred_capability as CapabilityCategory | null | undefined;
      if (!cap) return false;
      return !existing.has(`${cap}\u0000${m.id}`);
    }).length;
  }, [models, entries]);

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
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <button
            type="button"
            className="btn sm primary"
            onClick={() => void autoFetchAndFillAll()}
            disabled={disabled || autoFillAllLoading}
            title="自动获取模型列表, 并按推断一次性填入全部 5 个能力分类"
          >
            <Sparkles size={12} />
            {autoFillAllLoading
              ? "获取中..."
              : totalAutoFillCandidates > 0
                ? `自动获取并填充 (${totalAutoFillCandidates})`
                : "自动获取并填充"}
          </button>
          <button
            type="button"
            className="btn sm"
            onClick={() => (externalTest ? externalTest.run(active) : void runPoolTest())}
            disabled={disabled || testRunning}
            title={`逐个测试 ${CAPABILITY_LABEL[active]} 分类下所有模型的连通性`}
          >
            <RefreshCw size={12} className={testRunning ? "animate-spin" : undefined} />
            {testRunning
              ? `测试中 ${testProg?.done ?? 0}/${testProg?.total ?? 0}...`
              : `全部测试 ${CAPABILITY_LABEL[active]}`}
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

      {/* 模型获取状态 + 一键填充结果 */}
      <div className="field-hint" style={{ marginBottom: 6, marginTop: 0 }}>
        {loading
          ? "正在获取模型列表..."
          : models
            ? `已获取 ${models.length} 个模型`
            : "尚未获取模型列表 — 点「自动获取并填充」或「刷新模型列表」"}
        {autoFillAllMsg && (
          <span style={{ marginLeft: 8, color: "var(--ink-2, #6b7280)" }}>· {autoFillAllMsg}</span>
        )}
      </div>

      {/* 说明 */}
      <div className="field-hint" style={{ marginBottom: 10, marginTop: 0 }}>
        {CAPABILITY_LABEL[active]} 能力分类下的模型, 将以 <code>model-{active === "multimodal" ? "multimodal" : active}</code> 名称对外暴露.
        路由时按优先级 (数字最小优先) 取可用模型; 首选失败自动降级到下一个.
      </div>

      {/* 测试结果汇总条 — 只看当前 tab 的行; 逐行测试时数字会一个个涨上来 */}
      {(() => {
        const items = Object.entries(results)
          .filter(([k]) => k.startsWith(`${active}\u0000`))
          .map(([, v]) => v);
        if (items.length === 0) return null;
        const ok = items.filter((t) => t.enabled && t.ok).length;
        const fail = items.filter((t) => t.enabled && !t.ok).length;
        const skip = items.filter((t) => !t.enabled).length;
        const pending = testRunning && testProg ? testProg.total - testProg.done : 0;
        return (
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
            {pending > 0 && (
              <span style={{ color: "#6b7280" }}>
                <RefreshCw size={11} className="animate-spin" style={{ verticalAlign: -2, marginRight: 2 }} />
                待测 {pending}
              </span>
            )}
          </div>
        );
      })()}

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
            testItem={results[poolTestKey(active, entry.model_name)]}
            testing={activeTestingKey === poolTestKey(active, entry.model_name)}
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
  testing,
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
  /** 正在测这一行 (逐行测试时该行单独转圈) */
  testing?: boolean;
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
      {/* 连通性状态图标: 测试中→蓝圈转圈 / 未测试→虚线圈问号 / 成功→绿圆白勾 / 失败→红圆白叉 / 未启用→灰圆白减号 */}
      {testing ? (
        <div
          style={{
            width: 22,
            height: 22,
            borderRadius: 11,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            flexShrink: 0,
            border: "1.5px solid #93c5fd",
            background: "transparent",
          }}
          title="正在测试..."
        >
          <RefreshCw size={12} className="animate-spin" color="#3b82f6" />
        </div>
      ) : !testItem ? (
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

      {/* 模型输入: 可编辑下拉框 (input 永远可自由输入, 发现列表只作建议) */}
      <div style={{ flex: 1, minWidth: 0, position: "relative" }}>
        <ModelNameCombobox
          value={entry.model_name}
          onChange={onModelNameChange}
          options={discoveryCandidates}
          disabled={disabled}
        />
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
        {(["text", "image", "audio", "document"] as InputModality[]).map((mod) => {
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
              {mod === "text" ? "文本" : mod === "image" ? "图片" : mod === "audio" ? "音频" : "文档"}
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

/**
 * 可编辑下拉框: 输入框永远可自由打字, 下拉列表只是建议。
 *
 * 之前用 Radix `<Select>` 有个硬伤 —— 它只允许从选项里挑, 不支持自由文本。
 * 一旦「模型发现列表」拉到过 (刷新模型列表 / 自动获取并填充), 模型名就变成纯下拉,
 * 上游 `/models` 没返回的模型、或自定义模型名都填不进去, 「手动输入」建的空行也只能下拉。
 *
 * 现在改成 input + 行内建议列表:
 *   - 任意文本都可输入 (自由文本即最终值, 不会被强制改回列表项);
 *   - 列表项作为建议, 按输入过滤, 支持 ↑/↓ 选择、回车确认、Esc 关闭;
 *   - 菜单绝对定位在输入框正下方 (必要时上翻), 随所在容器滚动, 不被 Dialog 模态层拦截。
 */
function ModelNameCombobox({
  value,
  onChange,
  options,
  disabled,
}: {
  value: string;
  onChange: (v: string) => void;
  options: ModelInfo[];
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [hover, setHover] = useState(-1);
  /** 视口下方放不下时向上弹 */
  const [flipUp, setFlipUp] = useState(false);
  /** 用户正在输入才按内容过滤; 刚聚焦时展示全部建议 (否则只会命中当前值那一条) */
  const [dirty, setDirty] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const query = value.trim().toLowerCase();
  const filtered = useMemo(() => {
    if (!dirty || !query) return options;
    return options.filter(
      (m) =>
        m.id.toLowerCase().includes(query) ||
        (m.display_name ?? "").toLowerCase().includes(query),
    );
  }, [options, query, dirty]);

  function openMenu() {
    const el = inputRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const menuMax = 240;
    // 下方放不下且上方空间更大 → 向上弹
    setFlipUp(window.innerHeight - r.bottom < menuMax && r.top > menuMax);
    setOpen(true);
    setHover(-1);
  }

  function closeMenu() {
    setOpen(false);
    setHover(-1);
  }

  function pick(m: ModelInfo) {
    onChange(m.id);
    setDirty(false);
    closeMenu();
  }

  function onKeyDown(e: ReactKeyboardEvent<HTMLInputElement>) {
    if (!open) {
      if (e.key === "ArrowDown" && options.length > 0) {
        e.preventDefault();
        openMenu();
      }
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHover((h) => Math.min(h + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHover((h) => Math.max(h - 1, 0));
    } else if (e.key === "Enter") {
      if (hover >= 0 && hover < filtered.length) {
        e.preventDefault();
        pick(filtered[hover]);
      } else {
        // 没有高亮项: 保留当前自由文本
        closeMenu();
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeMenu();
    }
  }

  // 点击外部 → 关闭菜单 (菜单绝对定位在行内, 随容器滚动, 无需额外监听 scroll/resize)
  useEffect(() => {
    if (!open) return;
    function onDocMouseDown(ev: MouseEvent) {
      const t = ev.target as HTMLElement | null;
      if (inputRef.current?.contains(t as Node)) return;
      if (t?.closest?.("[data-model-combobox-menu]")) return;
      closeMenu();
    }
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
  }, [open]);

  return (
    <>
      <input
        ref={inputRef}
        className="input mono"
        style={{ width: "100%", height: 30 }}
        value={value}
        onChange={(e) => {
          setDirty(true);
          onChange(e.target.value);
          if (!open && options.length > 0) openMenu();
        }}
        onFocus={() => {
          setDirty(false);
          if (options.length > 0) openMenu();
        }}
        onKeyDown={onKeyDown}
        placeholder="输入或选择上游模型名 (如 gpt-4o)"
        disabled={disabled}
        autoComplete="off"
        spellCheck={false}
        role="combobox"
        aria-expanded={open}
        aria-autocomplete="list"
      />
      {open && filtered.length > 0 && (
          <div
            data-model-combobox-menu
            style={{
              position: "absolute",
              left: 0,
              right: 0,
              ...(flipUp ? { bottom: "calc(100% + 4px)" } : { top: "calc(100% + 4px)" }),
              maxHeight: 240,
              overflowY: "auto",
              zIndex: 40,
              background: "var(--surface, #ffffff)",
              border: "1px solid var(--line-2, #c9c5ba)",
              borderRadius: 8,
              boxShadow: "0 8px 24px rgba(0, 0, 0, 0.18)",
              padding: 4,
            }}
          >
            {filtered.map((m, i) => (
              <div
                key={m.id}
                // 用 mousedown + preventDefault: 抢在 input blur 之前选中, 点击不丢焦点
                onMouseDown={(e) => {
                  e.preventDefault();
                  pick(m);
                }}
                onMouseEnter={() => setHover(i)}
                className="font-mono"
                style={{
                  padding: "6px 8px",
                  borderRadius: 6,
                  cursor: "pointer",
                  fontSize: 12,
                  lineHeight: 1.4,
                  background: i === hover ? "var(--surface-2, #f5f3ec)" : "transparent",
                  color: "var(--ink, #141413)",
                  whiteSpace: "nowrap",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                }}
                title={m.id}
              >
                {m.display_name || m.id}
                {m.display_name && m.display_name !== m.id && (
                  <span style={{ marginLeft: 6, opacity: 0.55, fontSize: 11 }}>{m.id}</span>
                )}
                {m.inferred_capability && (
                  <span style={{ marginLeft: 6, opacity: 0.55, fontSize: 11 }}>
                    · 推断: {CAPABILITY_LABEL[m.inferred_capability]}
                  </span>
                )}
              </div>
            ))}
          </div>
        )}
    </>
  );
}
