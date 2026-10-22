import { useState } from "react";
import { useT } from "@/i18n";
import { RequestLogsPage } from "@/routes/RequestLogs";
import { SubscriptionEventsList } from "@/components/SubscriptionEventsList";
import { SystemErrorsList } from "@/components/SystemErrorsList";

type Tab = "requests" | "subscriptionEvents" | "systemErrors";

export function LogsPage() {
  const { t } = useT();
  const [tab, setTab] = useState<Tab>("requests");

  return (
    <>
      <div className="tabs">
        <TabButton active={tab === "requests"} onClick={() => setTab("requests")}>
          {t("logs.tab.requests")}
        </TabButton>
        <TabButton
          active={tab === "subscriptionEvents"}
          onClick={() => setTab("subscriptionEvents")}
        >
          {t("logs.tab.subscriptionEvents")}
        </TabButton>
        <TabButton
          active={tab === "systemErrors"}
          onClick={() => setTab("systemErrors")}
        >
          {t("logs.tab.systemErrors")}
        </TabButton>
      </div>

      {tab === "requests" && <RequestLogsPage />}
      {tab === "subscriptionEvents" && <SubscriptionEventsList />}
      {tab === "systemErrors" && <SystemErrorsList />}
    </>
  );
}

function TabButton({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button type="button" onClick={onClick} className={active ? "tab active" : "tab"}>
      {children}
    </button>
  );
}
