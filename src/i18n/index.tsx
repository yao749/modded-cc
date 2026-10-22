import {
  createContext,
  useContext,
  useCallback,
  useEffect,
  useMemo,
  type ReactNode,
} from "react";
import { useSettings } from "@/hooks/useSettings";
import zh from "./locales/zh.json";
import en from "./locales/en.json";
import ja from "./locales/ja.json";

export type Locale = "zh" | "en" | "ja";
export type LanguagePref = "system" | "zh" | "en" | "ja";

const dictionaries: Record<Locale, Record<string, string>> = {
  zh: zh as Record<string, string>,
  en: en as Record<string, string>,
  ja: ja as Record<string, string>,
};

/**
 * 写到 `<html lang>` 的 BCP 47 标签. 界面语言与它不一致时, 日文里的汉字会按中文字形渲染
 * (字体按 lang 选字形), 浏览器还会提示「翻译此页」, 屏幕阅读器也会读错语言.
 */
const HTML_LANG: Record<Locale, string> = { zh: "zh-CN", en: "en", ja: "ja" };

/** Read system locale via webview navigator. zh* → zh, ja* → ja, otherwise → en. */
export function detectSystemLocale(): Locale {
  if (typeof navigator === "undefined") return "en";
  const lang =
    navigator.language || (navigator.languages && navigator.languages[0]) || "en";
  const lower = lang.toLowerCase();
  if (lower.startsWith("zh")) return "zh";
  if (lower.startsWith("ja")) return "ja";
  return "en";
}

function resolveLocale(pref: LanguagePref | undefined): Locale {
  if (!pref || pref === "system") return detectSystemLocale();
  return pref;
}

export type TParams = Record<string, string | number>;
export type TFunction = (key: string, params?: TParams) => string;

type I18nValue = {
  t: TFunction;
  locale: Locale;
};

const I18nContext = createContext<I18nValue>({
  t: (k) => k,
  locale: "en",
});

function applyParams(template: string, params: TParams): string {
  let out = template;
  for (const [k, v] of Object.entries(params)) {
    out = out.split(`{${k}}`).join(String(v));
  }
  return out;
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const { data: settings } = useSettings();
  const locale = resolveLocale(
    settings?.preferred_language as LanguagePref | undefined,
  );
  const dict = dictionaries[locale];

  useEffect(() => {
    document.documentElement.lang = HTML_LANG[locale];
  }, [locale]);

  const t = useCallback<TFunction>(
    (key, params) => {
      // en/ja 缺失的 key 先回退中文, 避免界面出现原始 key（zh-first 文案流程）.
      const tpl = dict[key] ?? dictionaries.zh[key] ?? key;
      return params ? applyParams(tpl, params) : tpl;
    },
    [dict],
  );

  const value = useMemo<I18nValue>(() => ({ t, locale }), [t, locale]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useT() {
  return useContext(I18nContext);
}
