import { TriangleAlert } from "lucide-react";
import { useT, type TFunction } from "@/i18n";
import { shiftedPorts } from "@/lib/proxyRestart";
import type { RestartProxyResult } from "@/types";
import type { SettingsForm } from "./useSettingsForm";

type Tone = "" | "warn" | "err";

/** 代理卡片底部的提示条: 有未生效改动 / 代理未运行 / 上一次重启的结果, 附「重启 / 启动代理服务」按钮. */
export function ProxyRestartNotice({ form }: { form: SettingsForm }) {
  const { t } = useT();
  const status = form.proxy.data;
  if (!status) return null;
  const stopped = !status.running;

  const stoppedNotice: [Tone, string] = [
    "err",
    status.last_error
      ? t("settings.proxy.restart.stopped", { error: status.last_error })
      : t("settings.proxy.restart.stoppedNoReason"),
  ];
  const pendingNotice: [Tone, string] = ["warn", t("settings.proxy.needsRestart")];
  // 「已停止」的结果在代理已经跑起来 (比如另一端启动了它) 之后就过期了, 走实时状态分支
  const result =
    form.restartResult?.outcome === "stopped" && !stopped ? null : form.restartResult;

  // 实时状态优先于上一次重启的结果: 结果可能已过期 (之后代理崩溃 / 另一端改了配置),
  // 不能让「已按新配置运行」的旧文案盖住真实状态.
  let notice: [Tone, string];
  if (form.restarting) {
    notice = ["warn", t("settings.proxy.restart.running")];
  } else if (stopped && result?.outcome !== "stopped") {
    notice = stoppedNotice;
  } else if (form.restartPending && result?.outcome === "applied") {
    notice = pendingNotice;
  } else if (result) {
    notice = describeResult(result, t);
  } else if (stopped) {
    notice = stoppedNotice;
  } else if (form.restartPending) {
    notice = pendingNotice;
  } else {
    return null;
  }
  const [tone, text] = notice;

  const showButton = form.restarting || stopped || form.restartPending;
  return (
    <div className={tone ? `alert ${tone}` : "alert"}>
      <TriangleAlert size={14} />
      <span style={{ flex: 1 }}>{text}</span>
      {showButton && (
        <button
          className="btn"
          type="button"
          disabled={form.restarting}
          onClick={() => void form.restartProxy()}
        >
          {stopped ? t("settings.proxy.restart.start") : t("settings.proxy.restart.button")}
        </button>
      )}
    </div>
  );
}

function describeResult(r: RestartProxyResult, t: TFunction): [Tone, string] {
  switch (r.outcome) {
    case "applied": {
      const shifts = shiftedPorts(r.status);
      if (shifts.length === 0) {
        return ["", t("settings.proxy.restart.applied", { url: r.status.base_url })];
      }
      const moved = shifts
        .map(([from, to]) => t("settings.proxy.restart.shifted", { from, to }))
        .join(" ");
      return ["warn", `${moved} ${t("settings.proxy.restart.useUrl", { url: r.status.base_url })}`];
    }
    case "rolled_back":
      return ["warn", t("settings.proxy.restart.rolledBack", { error: r.error })];
    case "stopped":
      return ["err", t("settings.proxy.restart.stoppedAfter", { error: r.error })];
    case "failed":
      return ["warn", t("settings.proxy.restart.notRun", { error: r.error })];
  }
}
