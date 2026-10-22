import { WindowChrome } from "@/components/layout/WindowChrome";
import { useNavigate } from "react-router";
import { TriangleAlert } from "lucide-react";
import { LogoMark } from "@/components/sketch/LogoMark";
import { useT } from "@/i18n";
import { runtime } from "@/runtime";

const DISCLAIMER_FLAG_KEY = "cc-router.disclaimer-accepted";

export function OnboardingDisclaimerPage() {
  const { t } = useT();
  const navigate = useNavigate();

  function handleAccept() {
    try {
      localStorage.setItem(DISCLAIMER_FLAG_KEY, "1");
    } catch {
      // localStorage 写失败时仍允许继续, 下次启动会再被拦一次, 可接受
    }
    navigate("/subscriptions/new?onboarding=1", { replace: true });
  }

  return (
    <div className="onboarding-disclaimer-shell">
      {/* 无壳的全屏页: 同样铺拖窗带, 否则 Windows / Linux 上无法拖动与关闭 */}
      {runtime.kind === "desktop" && <WindowChrome />}
      <div className="card onboarding-disclaimer">
        <LogoMark size={72} plainSize={56} tile label="cc-router" className="onboarding-disclaimer-mark" />
        <h1 className="onboarding-disclaimer-title">
          <TriangleAlert size={18} />
          {t("onboarding.disclaimer.title")}
        </h1>
        <div className="onboarding-disclaimer-subtitle">
          {t("onboarding.disclaimer.subtitle")}
        </div>
        <div className="onboarding-disclaimer-body">
          <p>{t("about.disclaimer.usage")}</p>
          <p>{t("about.disclaimer.tos")}</p>
          <p>{t("about.disclaimer.warranty")}</p>
          <p>{t("about.disclaimer.affiliation")}</p>
        </div>
        <button
          className="btn primary onboarding-disclaimer-accept"
          type="button"
          onClick={handleAccept}
        >
          {t("onboarding.disclaimer.accept")}
        </button>
      </div>
    </div>
  );
}
