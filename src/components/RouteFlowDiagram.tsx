import { useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { stateLabel } from "@/components/StatusBadge";
import { ProviderLogo } from "@/components/ProviderLogo";
import { useProxyStatus } from "@/hooks/useSettings";
import { useVirtualModels } from "@/hooks/useVirtualModels";
import { useSubscriptions } from "@/hooks/useSubscriptions";
import { useProviders } from "@/hooks/useProviders";
import { useAnyRouteFlashState } from "@/hooks/useRouteFlash";
import { useClientActivity } from "@/hooks/useClientActivity";
import { fmtCooldownLeft } from "@/lib/format";
import { summarizeActivity, CLIENT_GROUP_LABEL, type ClientGroupKey } from "@/lib/clientActivity";
import { isCustomProviderId } from "@/lib/providerLabels";
import { VM_ORDER } from "@/lib/virtualModels";
import { useT, type TFunction } from "@/i18n";
import { LogoMark } from "@/components/sketch/LogoMark";
import { ClientDoodle, type ClientDoodleName } from "@/components/sketch/ClientDoodle";
import type { SubscriptionDto, VirtualModelDto } from "@/types";

/* ============================================================
 * 画布几何 (与 styles.css 的 .rf-* 绝对定位一体, 改一处必须改另一处)
 *
 *   0          14..170          436..524 (hub)          802..954     960
 *   ├─ 列标签 36px ─────────────────────────────────────────────────┤
 *   │  客户端便签 ─── 弧 ───▶ ■ ─── 弧 ───▶ 吊牌 (挂绳接住弧线末端)  │
 *
 * 画布宽度写死 960, 由 RouteFlowDiagram 按容器实测宽度整体等比缩小 (不横向滚动 ——
 * 拓扑图的价值在于一眼看全形状)。高度不是常数: 上游超过 6 家时按需增高, 经 --rf-h
 * 传给 CSS; 客户端 / hub / 弧线汇聚点全部相对「标签行以下的区域」垂直居中。
 * ============================================================ */
const CANVAS_W = 960;
/** 设计稿高度, 同时是下限 */
const CANVAS_H_MIN = 380;
/** 顶部列标签占的高度, 节点区从它下面开始算中心 */
const LABEL_H = 36;
const PAD_BOTTOM = 16;

const HUB = 88;
const HUB_LEFT = (CANVAS_W - HUB) / 2;
/** 客户端弧线的汇聚点 / 上游弧线的起点: 各离 hub 边缘 8px */
const HUB_IN = HUB_LEFT - 8;
const HUB_OUT = HUB_LEFT + HUB + 8;

const NOTE_LEFT = 14;
const NOTE_W = 156;
const NOTE_H = 58;
const CLIENT_STEP = 76;
const CLIENT_OUT = NOTE_LEFT + NOTE_W + 2;

const TAG_W = 152;
const TAG_H = 48;
/** 同列相邻吊牌的垂直步长: 吊牌高 48 + 8 间隙 */
const UP_STEP = 56;
const TAG_LEFT = CANVAS_W - 6 - TAG_W;
/** 弧线终点停在吊牌左边之前, 中间这段由吊牌自己的挂绳接上 */
const ARC_END = TAG_LEFT - 10;

/** 节点区的垂直中心 */
const midY = (h: number) => LABEL_H + (h - LABEL_H) / 2;

/**
 * 吊牌只排一列, 家数多了画布变高 (7 家起)。
 * 以前的云朵在 7 家以上左右交错两列, 但吊牌更宽, 内列会被外列的弧线横穿 —— 单列 + 增高
 * 没有这个问题, 也不用再为两列分别调控制点。
 */
function layoutUpstreams(count: number): { cys: number[]; height: number } {
  const span = UP_STEP * Math.max(0, count - 1);
  const height = Math.max(CANVAS_H_MIN, LABEL_H + span + TAG_H + PAD_BOTTOM);
  const mid = midY(height);
  return {
    cys: Array.from({ length: count }, (_, i) => mid - span / 2 + i * UP_STEP),
    height,
  };
}

const clientArc = (cy: number, hubCy: number) =>
  `M${CLIENT_OUT} ${cy} C 290 ${cy}, 330 ${hubCy}, ${HUB_IN} ${hubCy}`;
const upstreamArc = (hubCy: number, cy: number) =>
  `M${HUB_OUT} ${hubCy} C 640 ${hubCy}, 690 ${cy}, ${ARC_END} ${cy}`;
/** 手画小折线箭头, 尖端在 (x, y) */
const arrowHead = (x: number, y: number) => `M${x - 10} ${y - 5} L${x} ${y} L${x - 10} ${y + 5}`;

/**
 * 本地 AI Agent 工具便签。名字取自 CLIENT_GROUP_LABEL (与「客户端接入」卡的分组名同源),
 * 命令写死 —— 这里表达的是「谁可以调进来」, 便签亮/灰由「客户端接入」的被动流量检测
 * 决定 (useClientActivity): 保留期内发过请求的便签保持原色, 从未出现的整张灰掉。
 * 检测未出结果前 (加载/失败) 一律不灰, 免得每次进页面都闪一下。
 * 便签的颜色 / 角度 / 胶带位置逐张错开, 同一个值会显得像盖章。
 */
const CLIENTS: {
  name: string;
  /** 该便签对应的流量检测分组 (lib/clientActivity.ts), 与「客户端接入」卡同一套口径 */
  group: ClientGroupKey;
  /** null = 用本地化的「任何兼容客户端」 */
  cmd: string | null;
  icon: ClientDoodleName;
  fill: string;
  rotate: number;
  tapeLeft: number;
  tapeRotate: number;
}[] = [
  { group: "claude", name: CLIENT_GROUP_LABEL.claude, cmd: "$ claude", icon: "terminal", fill: "var(--fill-cactus)", rotate: -2.2, tapeLeft: 60, tapeRotate: 4 },
  { group: "codex", name: CLIENT_GROUP_LABEL.codex, cmd: "$ codex", icon: "braces", fill: "var(--fill-butter)", rotate: 1.6, tapeLeft: 24, tapeRotate: -5 },
  { group: "opencode", name: CLIENT_GROUP_LABEL.opencode, cmd: "$ opencode", icon: "laptop", fill: "var(--fill-sky)", rotate: -1, tapeLeft: 94, tapeRotate: 3 },
  { group: "others", name: CLIENT_GROUP_LABEL.others, cmd: null, icon: "bubble", fill: "var(--fill-coral)", rotate: 2.2, tapeLeft: 56, tapeRotate: -3 },
];

/**
 * 吊牌错位色块的「身份色」, 按上游在图上的顺序轮流取, 保证相邻两张不同色。
 * 奶油黄 (处理中) 与珊瑚色 (冷却中) 是状态色, 刻意不在这张表里。
 */
const TAG_FILLS = ["var(--fill-sky)", "var(--fill-heather)", "var(--fill-cactus)", "var(--fill-oat)", "var(--fill-stone)"];

/** ok = 可调度; err = 有订阅故障或冷却中; off = 名下订阅全被用户停用 */
type UpstreamTone = "ok" | "err" | "off";

interface UpstreamNode {
  /** 聚合 key: 内置 provider = provider_id; 自定义订阅 = provider_id + 订阅 id (每条独立成一张吊牌) */
  key: string;
  name: string;
  icon?: string;
  tone: UpstreamTone;
  /** err 时的状态名, 如「限流」 */
  statusLabel: string | null;
  /** err 且在冷却时的剩余时间, 如「4m」; 单独成段, 窄处只截状态名、不截倒计时 */
  cooldown: string | null;
  /** 该 provider 名下所有在用订阅 id, 用于实时闪烁聚合 */
  subIds: string[];
  /** 悬停提示: 名下每条订阅一行 */
  tooltip: string;
}

/**
 * 按容器实测宽度算缩放比 (只缩不放) 与居中偏移。
 * ResizeObserver 而不是媒体查询 —— 不再与侧栏宽度耦合。
 */
function useFitScale(width: number) {
  const ref = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState(width);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => setBox(el.clientWidth);
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const scale = Math.min(1, box / width);
  return { ref, scale, offset: Math.max(0, (box - width * scale) / 2) };
}

export function RouteFlowDiagram() {
  const { t } = useT();
  const proxy = useProxyStatus();
  const vms = useVirtualModels();
  const subs = useSubscriptions();
  const providers = useProviders();
  const { ref: fitRef, scale, offset } = useFitScale(CANVAS_W);
  const activity = useClientActivity();
  const summary = summarizeActivity(activity.data);

  const subsMap = useMemo(() => {
    const m = new Map<string, SubscriptionDto>();
    subs.data?.forEach((s) => m.set(s.id, s));
    return m;
  }, [subs.data]);

  const orderedVms = useMemo<VirtualModelDto[]>(
    () =>
      VM_ORDER.map((name) => vms.data?.find((v) => v.name === name)).filter(
        (v): v is VirtualModelDto => v !== undefined,
      ),
    [vms.data],
  );

  const upstreams = useMemo<UpstreamNode[]>(
    () => collectUpstreams(orderedVms, subsMap, providers.data, t),
    [orderedVms, subsMap, providers.data, t],
  );

  const running = proxy.data?.running ?? false;
  const { cys: upCys, height: canvasH } = layoutUpstreams(upstreams.length);
  const hubCy = midY(canvasH);
  const slotCount = orderedVms.filter((v) => v.name !== "model-fallback" && v.name !== "model-jev").length;
  const address = (proxy.data?.base_url ?? "").replace(/^https?:\/\//, "");

  return (
    <section className="card rf-card">
      <div className="card-head">
        <div className="rf-head-lead">
          <span className="card-title">{t("liveRouting.diagram.title")}</span>
          <span className="card-sub">
            <HandDigits text={t("liveRouting.summary", { slots: slotCount, vendors: upstreams.length })} />
          </span>
        </div>
        {running ? (
          <span className="rf-status">
            <span className="rf-status-dot" />
            {t("sidebar.proxyRunning")}
          </span>
        ) : (
          <span className="rf-status off">
            <span className="rf-status-dot" />
            {t("liveRouting.proxyStopped")}
          </span>
        )}
      </div>

      {/* 缩放层: 实测容器宽度等比缩小, 外层高度跟着收, 不留空白 */}
      <div ref={fitRef} className="rf-fit" style={{ height: canvasH * scale }}>
        <div
          className={running ? "rf-canvas" : "rf-canvas off"}
          style={{ "--rf-h": `${canvasH}px`, transform: `scale(${scale})`, marginLeft: offset } as CSSProperties}
        >
          <div className="rf-col-label left">
            <span className="hand">{t("liveRouting.col.clients")}</span>
            <span>{t("liveRouting.col.clientsSub")}</span>
          </div>
          <div className="rf-col-label right">
            <span>{t("liveRouting.col.upstreamsSub")}</span>
            <span className="hand">{t("liveRouting.col.upstreams")}</span>
          </div>

          <svg className="rf-arcs" viewBox={`0 0 ${CANVAS_W} ${canvasH}`} fill="none" aria-hidden>
            <g filter="url(#ccr-rough-canvas)">
              {CLIENTS.map((_, i) => {
                const cy = hubCy + (i - 1.5) * CLIENT_STEP;
                const d = clientArc(cy, hubCy);
                return (
                  <g key={`c${i}`}>
                    <path className="rf-line" d={d} />
                    {running && <path className="rf-flow client" d={d} />}
                  </g>
                );
              })}
              <path className="rf-arrow" d={arrowHead(HUB_IN, hubCy)} />
              {upstreams.map((u, i) => (
                <UpstreamArc key={u.key} node={u} d={upstreamArc(hubCy, upCys[i])} end={upCys[i]} running={running} />
              ))}
            </g>
          </svg>

          {CLIENTS.map((c, i) => (
            <ClientNote
              key={c.name}
              client={c}
              cmd={c.cmd ?? t("liveRouting.clientAny")}
              top={hubCy + (i - 1.5) * CLIENT_STEP - NOTE_H / 2}
              dormant={summary !== null && !summary.groups[c.group].active}
            />
          ))}

          <div className="rf-hub-note hand" style={{ top: hubCy - 99 }}>
            {t("liveRouting.hubNote")}
          </div>
          <svg className="rf-hub-note-arrow" viewBox="0 0 30 34" width="30" height="34" style={{ top: hubCy - 81 }} aria-hidden>
            <path d="M6 2 C 16 6, 20 16, 14 30 M8 24 L14 31 L19 23" />
          </svg>
          <div className="rf-hub" style={{ top: hubCy - HUB / 2 }}>
            <LogoMark size={HUB} tile label="cc-router" />
          </div>
          <div className="rf-hub-label" style={{ top: hubCy + HUB / 2 + 9 }}>
            <div className="rf-hub-name">cc-router</div>
            {address && <div className="rf-hub-addr">{address}</div>}
          </div>

          {upstreams.map((u, i) => (
            <UpstreamTag
              key={u.key}
              node={u}
              top={upCys[i] - TAG_H / 2}
              fill={TAG_FILLS[i % TAG_FILLS.length]}
            />
          ))}
        </div>
      </div>

      <div className="rf-legend">
        <span>
          <svg viewBox="0 0 28 8" width="28" height="8" aria-hidden>
            <path className="rf-line" d="M1 4 L27 4" />
            <path className="rf-flow upstream still" d="M1 4 L27 4" />
          </svg>
          {t("liveRouting.legend.ok")}
        </span>
        <span>
          <svg viewBox="0 0 28 8" width="28" height="8" aria-hidden>
            <path className="rf-flow active still" d="M1 4 L27 4" />
          </svg>
          <i className="rf-legend-swatch" />
          {t("liveRouting.legend.active")}
        </span>
        <span>
          <svg viewBox="0 0 28 8" width="28" height="8" aria-hidden>
            <path className="rf-line err" d="M1 4 L27 4" />
          </svg>
          {t("liveRouting.legend.cooling")}
        </span>
        <span>
          <i className="rf-legend-swatch dormant" />
          {t("liveRouting.legend.dormant")}
        </span>
        <span className="rf-legend-hint hand">{t("liveRouting.legend.hint")}</span>
      </div>
    </section>
  );
}

/** 把文案里的数字换成手写体 (「4 槽位 + 1 兜底 · 5 家在用」), 与语言无关 */
function HandDigits({ text }: { text: string }) {
  return (
    <>
      {text.split(/(\d+)/).map((part, i) =>
        /^\d+$/.test(part) ? (
          <b className="hand-num" key={i}>
            {part}
          </b>
        ) : (
          part
        ),
      )}
    </>
  );
}

function ClientNote({
  client,
  cmd,
  top,
  dormant,
}: {
  client: (typeof CLIENTS)[number];
  cmd: string;
  top: number;
  /** 保留期内没见过请求 → 灰掉; 加载中/失败为 false, 不灰 */
  dormant: boolean;
}) {
  return (
    <div className={dormant ? "rf-note dormant" : "rf-note"} style={{ top, transform: `rotate(${client.rotate}deg)` }}>
      <svg viewBox={`0 0 ${NOTE_W} ${NOTE_H}`} width={NOTE_W} height={NOTE_H} aria-hidden>
        <g filter="url(#ccr-rough-canvas)">
          {/* fill 是内联的, CSS 盖不住 —— 灰显色必须在组件里选 */}
          <path className="rf-note-paper" style={{ fill: dormant ? "var(--fill-stone)" : client.fill }} d="M2 2 L154 3 L153 44 L140 56 L3 56 Z" />
          <path className="rf-note-fold" d="M153 44 L141 45.5 L140 56 Z" />
        </g>
      </svg>
      <span className="rf-note-tape" style={{ left: client.tapeLeft, transform: `rotate(${client.tapeRotate}deg)` }} />
      <div className="rf-note-body">
        <ClientDoodle name={client.icon} />
        <div className="rf-note-text">
          <span className="rf-note-name">{client.name}</span>
          <span className="rf-note-cmd">{cmd}</span>
        </div>
      </div>
    </div>
  );
}

function UpstreamArc({ node, d, end, running }: { node: UpstreamNode; d: string; end: number; running: boolean }) {
  const active = useAnyRouteFlashState(node.subIds) !== undefined;
  // 异常 / 停用的链路不走流动层 —— 上面没有流量
  if (node.tone !== "ok") {
    return (
      <g>
        <path className={node.tone === "err" ? "rf-line err" : "rf-line off"} d={d} />
        <path className={node.tone === "err" ? "rf-arrow err" : "rf-arrow off"} d={arrowHead(ARC_END, end)} />
      </g>
    );
  }
  return (
    <g>
      <path className="rf-line" d={d} />
      {running && <path className={active ? "rf-flow active" : "rf-flow upstream"} d={d} />}
      <path className="rf-arrow" d={arrowHead(ARC_END, end)} />
    </g>
  );
}

function UpstreamTag({ node, top, fill }: { node: UpstreamNode; top: number; fill: string }) {
  const { t } = useT();
  const flash = useAnyRouteFlashState(node.subIds);
  // 实时高亮: 有请求打到这家时色块换奶油黄、描边加粗, 与弧线的陶土色墨点呼应
  const active = flash !== undefined && node.tone === "ok";
  const offset = node.tone === "err" ? "var(--fill-coral)" : node.tone === "off" ? "transparent" : active ? "var(--fill-butter)" : fill;

  return (
    <div className={`rf-tag ${node.tone}${active ? " active" : ""}`} style={{ top, left: TAG_LEFT }} title={node.tooltip}>
      <svg viewBox={`0 0 ${TAG_W} ${TAG_H}`} width={TAG_W} height={TAG_H} aria-hidden>
        <path className="rf-tag-offset" transform="translate(5 4)" style={{ fill: offset }} d={TAG_PATH} />
        <g className="rf-tag-ink" filter="url(#ccr-rough-canvas)">
          <path className="rf-tag-paper" d={TAG_PATH} />
          <circle className="rf-tag-hole" cx="15" cy="24" r="3.4" />
        </g>
        <path className="rf-tag-string" d="M-6 24 C 0 16, 10 16, 15 24" />
      </svg>
      <div className="rf-tag-body">
        <ProviderLogo iconId={node.icon} size={22} iconSize={14} />
        <div className="rf-tag-text">
          <span className="rf-tag-name">{node.name}</span>
          {node.tone === "ok" ? (
            <span className="rf-tag-sub">
              {t(node.subIds.length === 1 ? "liveRouting.subCountOne" : "liveRouting.subCount", { n: node.subIds.length })}
            </span>
          ) : (
            <span className="rf-tag-state hand">
              <span className="rf-tag-state-label">
                {node.tone === "err" ? node.statusLabel : stateLabel("disabled", t)}
              </span>
              {node.tone === "err" && node.cooldown && <span>{node.cooldown}</span>}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}

/** 吊牌轮廓: 左端是穿挂绳的尖角 */
const TAG_PATH = "M16 2 L150 3 L151 45 L16 46 L2 24 Z";

/**
 * 一家 provider 名下的订阅 → 一张吊牌的状态。
 *
 * - 停用是用户的主动选择, 不是故障: 只有「全部停用」才整张灰掉; 部分停用时忽略停用的那几条。
 * - 其余订阅里只要有一条非 healthy 就整张标异常 —— 一张吊牌没有「半健康」的表达方式,
 *   报警比报平安安全。
 */
function classifyUpstream(list: SubscriptionDto[]): { tone: UpstreamTone; bad?: SubscriptionDto } {
  const enabled = list.filter((s) => s.state !== "disabled");
  if (enabled.length === 0) return { tone: "off" };
  const bad = enabled.find((s) => s.state !== "healthy");
  return bad ? { tone: "err", bad } : { tone: "ok" };
}

/**
 * 收集「在用」的上游: 只算被虚拟模型引用到的订阅, 按 provider 去重。
 *
 * 自定义订阅例外: 它们共享同一个 marker 作 provider_id (custom / custom-openai / ...),
 * 按 provider 聚合会把互不相关的端点并成一张吊牌, 状态互相污染 (issue #40) ——
 * 改为每条自定义订阅独立成一张, 名字取订阅自己的 provider_display_name。
 */
function collectUpstreams(
  vms: VirtualModelDto[],
  subsMap: Map<string, SubscriptionDto>,
  providers: { id: string; display_name: string; icon?: string }[] | undefined,
  t: TFunction,
): UpstreamNode[] {
  const byProvider = new Map<string, SubscriptionDto[]>();
  for (const vm of vms) {
    for (const sid of vm.subscription_ids) {
      const sub = subsMap.get(sid);
      if (!sub) continue;
      const groupKey = isCustomProviderId(sub.provider_id)
        ? `${sub.provider_id}:${sub.id}`
        : sub.provider_id;
      const list = byProvider.get(groupKey);
      if (list) {
        if (!list.some((s) => s.id === sub.id)) list.push(sub);
      } else {
        byProvider.set(groupKey, [sub]);
      }
    }
  }

  return Array.from(byProvider.entries()).map(([groupKey, list]) => {
    const info = providers?.find((p) => p.id === list[0].provider_id);
    const name = info?.display_name ?? list[0].provider_display_name ?? list[0].provider_id;
    const { tone, bad } = classifyUpstream(list);
    const cooldown = bad ? fmtCooldownLeft(bad.cooldown_until) : null;
    return {
      key: groupKey,
      name,
      icon: info?.icon ?? list[0].provider_icon,
      tone,
      statusLabel: bad ? stateLabel(bad.state, t) : null,
      cooldown,
      subIds: list.map((s) => s.id),
      tooltip: [name, ...list.map((s) => `· ${s.display_name} — ${stateLabel(s.state, t)}`)].join("\n"),
    };
  });
}
