import { Star } from "lucide-react";
import { runtime } from "@/runtime";
import { useT } from "@/i18n";
import { cn } from "@/lib/utils";

export const REPO_URL = "https://github.com/finch-xu/cc-router";

/**
 * 求 Star 提示条: 陶土浅底, 文案左、按钮右, 窄窗口时按钮折到下一行。
 * 关于页卡片底部与虚拟模型页底部共用, 两处必须长得一样, 所以收成一个组件。
 */
export function StarPrompt({ className }: { className?: string }) {
  const { t } = useT();
  return (
    <div className={cn("star-prompt", className)}>
      <Star size={14} className="star-prompt-icon" />
      <span className="star-prompt-text">{t("about.star.text")}</span>
      <button
        className="btn primary"
        type="button"
        onClick={() => runtime.openExternal(REPO_URL).catch(() => {})}
      >
        <Star size={12} /> {t("about.star.action")}
      </button>
    </div>
  );
}
