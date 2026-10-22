import type { NotesDoc, NotesLang, VersionNotes } from "@/types";

/** 标签顺序固定; 标签名用各自的母语, 不随界面语言翻译 (spec §5.3) */
export const NOTES_LANGS: readonly NotesLang[] = ["zh", "en", "ja"];

export const NOTES_LANG_LABEL: Record<NotesLang, string> = {
  zh: "中文",
  en: "English",
  ja: "日本語",
};

/** 该版本实际写了哪些语言, 按固定顺序 */
export function availableLangs(v: VersionNotes): NotesLang[] {
  return NOTES_LANGS.filter((l) => v.notes[l] !== undefined);
}

/** 想看 want; 没有就按 en → zh 回退。zh 必有, 所以总有结果 */
export function resolveLang(want: NotesLang, v: VersionNotes): NotesLang {
  for (const l of [want, "en", "zh"] as const) {
    if (v.notes[l] !== undefined) return l;
  }
  return "zh";
}

export function docFor(v: VersionNotes, lang: NotesLang): NotesDoc {
  return v.notes[lang] ?? v.notes.zh;
}
