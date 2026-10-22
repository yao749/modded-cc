import { useState } from "react";
import { useT } from "@/i18n";
import type { NotesLang, VersionNotes } from "@/types";
import { NotesDocView } from "./NotesDocView";
import { docFor, resolveLang } from "./pickLang";

/** 折叠行里的一句概要: 第一个分节前两条列表项开头的粗体要点名 */
function teaserOf(v: VersionNotes, lang: NotesLang): string {
  const items = docFor(v, lang).sections[0]?.items ?? [];
  return items
    .slice(0, 2)
    .map((it) => it.text.find((p) => p.kind === "bold")?.text)
    .filter((s): s is string => Boolean(s))
    .join(" · ");
}

/** 折叠的旧版本: 展开后按当前标签的语言显示; 该版本没有这种语言时按 en → zh 回退并给提示条 */
export function VersionRow({ v, lang }: { v: VersionNotes; lang: NotesLang }) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  const shown = resolveLang(lang, v);
  return (
    <div className={open ? "rn-row open" : "rn-row"}>
      <button type="button" className="rn-row-btn" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <svg className={open ? "rn-chev open" : "rn-chev"} viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <path d="M5.6 3.2 C 7.6 5, 9.2 6.6, 10.6 8 C 9.2 9.4, 7.6 11, 5.4 12.8" />
        </svg>
        <span className="rn-row-ver">v{v.version}</span>
        <span className="rn-row-teaser">{teaserOf(v, shown)}</span>
        {v.date && <span className="rn-row-date">{v.date}</span>}
      </button>
      {open && (
        <div className="rn-row-body">
          {shown !== lang && (
            <div className="rn-fallback">
              {t("releaseNotes.fallback", {
                lang: t(`releaseNotes.lang.${lang}`),
                shown: t(`releaseNotes.lang.${shown}`),
              })}
            </div>
          )}
          <NotesDocView doc={docFor(v, shown)} />
        </div>
      )}
    </div>
  );
}
