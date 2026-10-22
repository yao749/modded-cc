import { useState } from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import * as TabsPrimitive from "@radix-ui/react-tabs";
import { cn } from "@/lib/utils";
import { useT } from "@/i18n";
import { PartyPopper } from "lucide-react";
import { SidebarIcon } from "@/components/sketch/SidebarIcon";
import { PixelIcon } from "@/components/win2k/PixelIcon";
import { useTheme } from "@/hooks/useTheme";
import type { NotesLang, VersionNotes } from "@/types";
import { NotesDocView } from "./NotesDocView";
import { VersionRow } from "./VersionRow";
import { NOTES_LANG_LABEL, availableLangs, docFor, resolveLang } from "./pickLang";

/** auto = 升级后自动弹出; manual = 从侧栏打开 */
export type ReleaseNotesMode = "auto" | "manual";

interface Props {
  open: boolean;
  onClose: () => void;
  mode: ReleaseNotesMode;
  /** 展开的版本 */
  main: VersionNotes;
  /** 折叠在下半部分的版本: auto = 期间错过的, manual = 更早的 */
  others: VersionNotes[];
}

/**
 * 「更新内容」弹窗 (spec §5.3 / §5.4, 视觉稿见 spec 头部链接)。
 * 直接用 Radix 原语而不是 ui/dialog.tsx 的 DialogContent: 后者自带的右上角 × 和 max-w-lg 与标签栏布局冲突。
 * 标签只切换说明正文; 其余文字跟随界面语言。每次打开由父组件换 key 重新挂载, 语言因此回到界面语言。
 */
export function ReleaseNotesDialog({ open, onClose, mode, main, others }: Props) {
  const { t, locale } = useT();
  const { art } = useTheme();
  const langs = availableLangs(main);
  const [lang, setLang] = useState<NotesLang>(() => resolveLang(locale, main));

  return (
    <DialogPrimitive.Root open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="cc-dialog-overlay rn-overlay" />
        <DialogPrimitive.Content className="rn" aria-describedby={undefined}>
          <TabsPrimitive.Root value={lang} onValueChange={(v) => setLang(v as NotesLang)} className="rn-main">
            <div className="rn-top">
              {langs.length > 1 ? (
                <TabsPrimitive.List className="rn-tabs" aria-label={t("releaseNotes.langs")}>
                  {langs.map((l) => (
                    <TabsPrimitive.Trigger key={l} value={l} className={cn("tab", lang === l && "active")}>
                      {NOTES_LANG_LABEL[l]}
                    </TabsPrimitive.Trigger>
                  ))}
                </TabsPrimitive.List>
              ) : (
                <div className="rn-tabs-spacer" />
              )}
              <DialogPrimitive.Close className="rn-x" aria-label={t("releaseNotes.close")}>
                <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
                  <path d="M3.6 3.4 C 6.8 6.4, 9.6 9.4, 12.6 12.6 M12.4 3.6 C 9.4 6.6, 6.6 9.6, 3.4 12.4" />
                </svg>
              </DialogPrimitive.Close>
            </div>
            <header className="rn-head">
              <div>
                <span className="rn-eyebrow">{t("releaseNotes.eyebrow")}</span>
                <DialogPrimitive.Title className="rn-title">cc-router {main.version}</DialogPrimitive.Title>
                {(main.codename || main.date) && (
                  <div className="rn-meta">
                    {main.codename && <span className="rn-code">{main.codename}</span>}
                    {main.date && <span className="rn-date">{main.date}</span>}
                  </div>
                )}
              </div>
              <div className="rn-doodle">
                {art === "sketch" ? (
                  <SidebarIcon name="whatsnew" size={74} />
                ) : art === "win2k" ? (
                  <PixelIcon name="whatsnew" size={32} />
                ) : (
                  <PartyPopper size={40} strokeWidth={1.5} aria-hidden="true" />
                )}
              </div>
            </header>
            <TabsPrimitive.Content value={lang} className="rn-body" tabIndex={-1}>
              <NotesDocView doc={docFor(main, lang)} />
              {others.length > 0 && (
                <>
                  <svg className="rn-rule" viewBox="0 0 240 10" preserveAspectRatio="none" aria-hidden="true">
                    <path d="M0 5 C 15 1, 45 9, 60 5 C 75 1, 105 9, 120 5 C 135 1, 165 9, 180 5 C 195 1, 225 9, 240 5" />
                  </svg>
                  <div className="rn-group">
                    {mode === "auto" ? t("releaseNotes.missed", { count: others.length }) : t("releaseNotes.earlier")}
                  </div>
                  <div className="rn-vers">
                    {others.map((v) => (
                      <VersionRow key={v.version} v={v} lang={lang} />
                    ))}
                  </div>
                </>
              )}
            </TabsPrimitive.Content>
          </TabsPrimitive.Root>
          <footer className="rn-foot">
            {/* 提示「可以从侧栏再打开」只在自动弹出时有意义 */}
            <span className="rn-hint">{mode === "auto" ? t("releaseNotes.hint") : ""}</span>
            <button type="button" className="btn primary" onClick={onClose}>
              {t("releaseNotes.ok")}
            </button>
          </footer>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}
