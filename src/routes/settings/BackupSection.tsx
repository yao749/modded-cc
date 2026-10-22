import { useState } from "react";
import { useT } from "@/i18n";
import { ExportConfigDialog } from "./ExportConfigDialog";
import { ImportConfigDialog } from "./ImportConfigDialog";

/** 设置 → 高级: 备份与迁移。放在危险区域之前。 */
export function BackupSection() {
  const { t } = useT();
  const [exportOpen, setExportOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);

  return (
    <>
      <div className="card section">
        <div className="card-head">
          <div className="card-title">{t("settings.section.backup")}</div>
        </div>
        <div className="card-body">
          <div className="field-hint" style={{ marginBottom: 10 }}>{t("settings.backup.desc")}</div>
          <div style={{ display: "flex", gap: 8 }}>
            <button className="btn" type="button" onClick={() => setExportOpen(true)}>
              {t("settings.backup.export")}
            </button>
            <button className="btn" type="button" onClick={() => setImportOpen(true)}>
              {t("settings.backup.import")}
            </button>
          </div>
        </div>
      </div>
      <ExportConfigDialog open={exportOpen} onOpenChange={setExportOpen} />
      <ImportConfigDialog open={importOpen} onOpenChange={setImportOpen} />
    </>
  );
}
