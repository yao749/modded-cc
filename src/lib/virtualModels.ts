import type {
  EndpointProtocol,
  RoutingMode,
  SubscriptionDto,
  SubscriptionSlot,
  VirtualModelName,
} from "@/types";

export const VM_ORDER: VirtualModelName[] = [
  "model-fable",
  "model-opus",
  "model-sonnet",
  "model-haiku",
  "model-fallback",
  "model-jev",
];

export interface VmMeta {
  /** i18n key for the "purpose" line shown above the slot */
  purposeKey: string;
  /** i18n key for the secondary English-style purpose tag */
  purposeEnKey: string;
  /** i18n key for the long human-readable label (used in dropdowns) */
  labelKey: string;
}

export const VM_META: Record<VirtualModelName, VmMeta> = {
  "model-fable": {
    purposeKey: "vm.fable.purpose",
    purposeEnKey: "vm.fable.purposeEn",
    labelKey: "vm.fable.label",
  },
  "model-opus": {
    purposeKey: "vm.opus.purpose",
    purposeEnKey: "vm.opus.purposeEn",
    labelKey: "vm.opus.label",
  },
  "model-sonnet": {
    purposeKey: "vm.sonnet.purpose",
    purposeEnKey: "vm.sonnet.purposeEn",
    labelKey: "vm.sonnet.label",
  },
  "model-haiku": {
    purposeKey: "vm.haiku.purpose",
    purposeEnKey: "vm.haiku.purposeEn",
    labelKey: "vm.haiku.label",
  },
  "model-fallback": {
    purposeKey: "vm.fallback.purpose",
    purposeEnKey: "vm.fallback.purposeEn",
    labelKey: "vm.fallback.label",
  },
  "model-jev": {
    purposeKey: "vm.jev.purpose",
    purposeEnKey: "vm.jev.purposeEn",
    labelKey: "vm.jev.label",
  },
};

/** fallback / jev 不绑四槽 (fallback 走原样透传, jev 走自己的 jev 槽) */
const SLOT_BY_VM: Record<VirtualModelName, SubscriptionSlot | null> = {
  "model-fable": "fable",
  "model-opus": "opus",
  "model-sonnet": "sonnet",
  "model-haiku": "haiku",
  "model-fallback": null,
  "model-jev": null,
};

export function vmNameToSlot(name: VirtualModelName): SubscriptionSlot | null {
  return SLOT_BY_VM[name];
}

export const MODE_LABEL_KEY: Record<RoutingMode, string> = {
  sequential: "vm.mode.sequential",
  round_robin: "vm.mode.round_robin",
  sticky: "vm.mode.sticky",
};

export function isJev(name: VirtualModelName): boolean {
  return name === "model-jev";
}

/**
 * 客户端写 model-jev 时这条订阅实际发给上游的模型, 与后端 systemone.rs::target_model 同一条规则:
 * Jev 槽 → 端点示例模型 (创建时快照进 model_discovery.example_models)。都没有时返回 null。
 */
export function jevModelOf(sub: Pick<SubscriptionDto, "model_slots" | "model_discovery">): string | null {
  return sub.model_slots.jev?.trim() || sub.model_discovery.example_models[0]?.trim() || null;
}

/** 与后端 VirtualModelName::accepts 同一条规则: model-jev 只收 systemone 订阅, 其余只收对话类订阅。 */
export function vmAccepts(name: VirtualModelName, protocol: EndpointProtocol): boolean {
  return isJev(name) ? protocol === "systemone" : protocol !== "systemone";
}
