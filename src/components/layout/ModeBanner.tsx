import { useLocation } from "react-router";
import { useQueryClient } from "@tanstack/react-query";
import { useSettings, useUpdateSettings } from "@/hooks/useSettings";
import { useT } from "@/i18n";
import type { GlobalRoutingMode } from "@/types";

/** 能力池组页面: 常显横幅 (本模式 = 弱提示, 非本模式 = 强警告) */
const POOL_ROUTES = ["/pool", "/pool/models"];
/** pool 模式下会误导用户的经典配置页 (四槽没生效) */
const CLASSIC_CONFIG_ROUTES = ["/virtual-models", "/subscriptions"];

function matches(pathname: string, patterns: string[]) {
  return patterns.some((p) => pathname === p || pathname.startsWith(`${p}/`));
}

/**
 * 路由模式门户横幅。
 *
 * 后端每请求读 settings.routing_mode, 所以这里的一键切换是热生效的 ——
 * 点完立即改 /v1/models 清单和 pipeline 门禁, 不需要重启代理服务。
 */
export function ModeBanner() {
  const { t } = useT();
  const { pathname } = useLocation();
  const settings = useSettings();
  const update = useUpdateSettings();
  const queryClient = useQueryClient();

  const mode: GlobalRoutingMode = settings.data?.routing_mode ?? "slots";
  const onPool = matches(pathname, POOL_ROUTES);
  const onClassicConfig = matches(pathname, CLASSIC_CONFIG_ROUTES);

  // 主场页面 (slots 模式下的经典配置页) 不出横幅
  if (!onPool && !(onClassicConfig && mode === "pool")) return null;

  // 池组页面在自己模式里只是弱提示, 其余都是「你看到的不是生效的那套」强警告
  const weak = onPool && mode === "pool";
  const other: GlobalRoutingMode = mode === "pool" ? "slots" : "pool";

  const textKey = onPool
    ? mode === "pool"
      ? "modeBanner.poolHome"
      : "modeBanner.poolNotActive"
    : "modeBanner.classicNotActive";

  function switchTo(next: GlobalRoutingMode) {
    update.mutate(
      { routing_mode: next },
      {
        onSuccess: () => {
          // 门禁/清单都按模式变, 三个列表一起刷
          queryClient.invalidateQueries({ queryKey: ["virtual-models"] });
          queryClient.invalidateQueries({ queryKey: ["model-pools"] });
          queryClient.invalidateQueries({ queryKey: ["subscriptions"] });
        },
      },
    );
  }

  return (
    <div className={weak ? "mode-banner weak" : "mode-banner strong"} role="status">
      <span className="mode-banner-text">{t(textKey)}</span>
      <button
        type="button"
        className="mode-banner-switch"
        disabled={update.isPending}
        onClick={() => switchTo(other)}
      >
        {t(other === "pool" ? "modeBanner.switchToPool" : "modeBanner.switchToSlots")}
      </button>
    </div>
  );
}
