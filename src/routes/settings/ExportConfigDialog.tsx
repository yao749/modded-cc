import { useEffect, useState } from "react";
import { Toggle } from "@/components/Toggle";
import { Spinner } from "@/components/Spinner";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { api } from "@/api/tauri";
import { useT } from "@/i18n";
import { runtime } from "@/runtime";
import { errorText } from "@/lib/errorText";
import { generateStrongPassword, MIN_PASSWORD_CHARS } from "@/lib/password";
import { localDayKey } from "@/lib/localDay";

export function ExportConfigDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (v: boolean) => void }) {
  const { t } = useT();
  const isDesktop = runtime.kind === "desktop";
  const [withSecrets, setWithSecrets] = useState(false);
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [generated, setGenerated] = useState<string | null>(null);
  const [copyStatus, setCopyStatus] = useState<"copied" | "failed" | null>(null);

  const tooShort = withSecrets && [...password].length < MIN_PASSWORD_CHARS;
  const mismatch = withSecrets && password !== confirm;
  const canSubmit = !busy && (!withSecrets || (!tooShort && !mismatch));

  function reset() {
    setWithSecrets(false);
    setPassword("");
    setConfirm("");
    setMessage(null);
    setError(null);
    setGenerated(null);
    setCopyStatus(null);
  }

  useEffect(() => {
    if (!open) reset();
  }, [open]);

  function fillGenerated() {
    const p = generateStrongPassword();
    setPassword(p);
    setConfirm(p);
    setGenerated(p);
    runtime.copyText(p)
      .then(() => setCopyStatus("copied"))
      .catch(() => setCopyStatus("failed"));
  }

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      const day = localDayKey(new Date());
      const filename = withSecrets ? `cc-router-backup-${day}-encrypted.json` : `cc-router-backup-${day}.json`;
      if (!isDesktop) {
        const text = await api.exportConfigText();
        runtime.downloadText(filename, text, "application/json;charset=utf-8");
        const count = (JSON.parse(text) as { subscriptions: unknown[] }).subscriptions.length;
        setMessage(t("backup.export.success", { count }));
        return;
      }
      const path = await runtime.pickSavePath({
        defaultName: filename,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path) return;
      const summary = await api.exportConfig(path, withSecrets ? password : undefined);
      setMessage(
        t(summary.with_secrets ? "backup.export.successWithSecrets" : "backup.export.success", {
          count: summary.subscriptions,
        }),
      );
    } catch (e) {
      setError(`${t("backup.export.failed")}: ${errorText(e)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(v) => {
        if (busy) return;
        onOpenChange(v);
      }}
    >
      <DialogContent className="cc-dialog">
        <DialogHeader>
          <DialogTitle>{t("backup.export.title")}</DialogTitle>
          <DialogDescription>{t("settings.backup.desc")}</DialogDescription>
        </DialogHeader>

        <div className="setting-row">
          <div className="label-col">{t("backup.export.includeSecrets")}</div>
          <Toggle
            checked={withSecrets}
            onChange={setWithSecrets}
            disabled={!isDesktop || busy}
            aria-label={t("backup.export.includeSecrets")}
          />
        </div>
        <div className="field-hint">
          {isDesktop ? t("backup.export.includeSecretsHint") : t("backup.export.webOnlyPlain")}
        </div>

        {withSecrets && (
          <div style={{ display: "flex", flexDirection: "column", gap: 8, marginTop: 12 }}>
            <input
              className="input mono"
              type="password"
              autoComplete="new-password"
              placeholder={t("backup.export.password")}
              value={password}
              onChange={(e) => {
                setPassword(e.target.value);
                setGenerated(null);
                setCopyStatus(null);
              }}
            />
            <input
              className="input mono"
              type="password"
              autoComplete="new-password"
              placeholder={t("backup.export.passwordConfirm")}
              value={confirm}
              onChange={(e) => {
                setConfirm(e.target.value);
                setGenerated(null);
                setCopyStatus(null);
              }}
            />
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <button className="btn sm" type="button" onClick={fillGenerated}>
                {t("backup.export.generate")}
              </button>
              {password && tooShort && (
                <span className="field-hint">{t("backup.export.passwordTooShort", { min: MIN_PASSWORD_CHARS })}</span>
              )}
              {!tooShort && confirm && mismatch && (
                <span className="field-hint">{t("backup.export.passwordMismatch")}</span>
              )}
            </div>
            {generated && (
              <>
                <div className="mono" style={{ userSelect: "all", padding: 8, backgroundColor: "var(--surface-2)", borderRadius: 4 }}>
                  {generated}
                </div>
                <div className="field-hint">
                  {copyStatus === "copied" && t("backup.export.copied")}
                  {copyStatus === "failed" && <span style={{ color: "var(--err)" }}>{t("backup.export.copyFailed")}</span>}
                </div>
              </>
            )}
            <div className="field-hint" style={{ color: "var(--warn)" }}>
              {t("backup.export.passwordWarning")}
            </div>
          </div>
        )}

        {message && <div className="field-hint" style={{ marginTop: 12 }}>{message}</div>}
        {error && <div className="field-hint" style={{ marginTop: 12, color: "var(--err)" }}>{error}</div>}

        <DialogFooter>
          <button className="btn" type="button" onClick={() => onOpenChange(false)} disabled={busy}>
            {t("common.close")}
          </button>
          <button className="btn primary" type="button" onClick={submit} disabled={!canSubmit}>
            {busy && <Spinner />}
            {t("backup.export.confirm")}
          </button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
