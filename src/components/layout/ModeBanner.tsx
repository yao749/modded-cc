import { useLocation } from "react-router";
import { useSettings } from "@/hooks/useSettings";
import { useT } from "@/i18n";
import type { GlobalRoutingMode } from "@/types";

/** pool 模式下会误导用户的经典配置页 (四槽被门禁拦, 没生效) */
const CLASSIC_CONFIG_ROUTES = ["/virtual-models", "/subscriptions"];

function matches(pathname: string, patterns: string[]) {
  return patterns.some((p) => pathname === p || pathname.startsWith(`${p}/`));
}

/**
 * 路由模式横幅 (只读, 不带切换按钮)。
 *
 * 只在 **pool 模式 + 经典配置页** 时警示「四槽没生效」。
 * 能力池组页面永不显示横幅 (2026-10-23 起): 模型池是独立体系 —— 有自己的订阅 /
 * 虚拟模型 / 总览, 任何模式下都能正常使用, 不存在「没生效」一说。
 */
export function ModeBanner() {
  const { t } = useT();
  const { pathname } = useLocation();
  const settings = useSettings();

  const mode: GlobalRoutingMode = settings.data?.routing_mode ?? "slots";
  const onClassicConfig = matches(pathname, CLASSIC_CONFIG_ROUTES);

  if (!(onClassicConfig && mode === "pool")) return null;

  return (
    <div className="mode-banner strong" role="status">
      <span className="mode-banner-text">{t("modeBanner.classicNotActive")}</span>
    </div>
  );
}
