import {
  createContext,
  useContext,
  useEffect,
  useState,
  useCallback,
  type ReactNode,
} from "react";
import {
  applyThemeAttrs,
  DEFAULT_THEME_ID,
  isThemeId,
  THEME_ID_KEY,
  themeDef,
  type ThemeArt,
  type ThemeId,
} from "@/themes";

/** 明暗模式 (与界面主题正交: 每个主题都有明暗两套) */
export type ColorMode = "light" | "dark" | "system";
type ResolvedMode = "light" | "dark";

// 键名沿用明暗模式最早的叫法, 改名会丢掉老用户的选择
const MODE_KEY = "cc-router-theme";

function readStorage(key: string): string | null {
  try {
    if (typeof window === "undefined") return null;
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStorage(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // localStorage 不可用时静默降级
  }
}

function getStoredMode(): ColorMode {
  const v = readStorage(MODE_KEY);
  return v === "light" || v === "dark" ? v : "system";
}

function getStoredThemeId(): ThemeId {
  const v = readStorage(THEME_ID_KEY);
  return isThemeId(v) ? v : DEFAULT_THEME_ID;
}

function getSystemDark(): boolean {
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  } catch {
    return false;
  }
}

interface ThemeContextValue {
  mode: ColorMode;
  resolved: ResolvedMode;
  setMode: (m: ColorMode) => void;
  themeId: ThemeId;
  /** 当前主题的画风; 按画风换组件 (侧栏图标 / Logo / 路由图) 时读它 */
  art: ThemeArt;
  setThemeId: (id: ThemeId) => void;
}

const ThemeContext = createContext<ThemeContextValue>({
  mode: "system",
  resolved: "light",
  setMode: () => {},
  themeId: DEFAULT_THEME_ID,
  art: themeDef(DEFAULT_THEME_ID).art,
  setThemeId: () => {},
});

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [mode, setModeState] = useState<ColorMode>(getStoredMode);
  const [themeId, setThemeIdState] = useState<ThemeId>(getStoredThemeId);
  const [systemDark, setSystemDark] = useState<boolean>(getSystemDark);

  const setMode = useCallback((m: ColorMode) => {
    writeStorage(MODE_KEY, m);
    setModeState(m);
  }, []);

  const setThemeId = useCallback((id: ThemeId) => {
    writeStorage(THEME_ID_KEY, id);
    setThemeIdState(id);
  }, []);

  const resolved: ResolvedMode =
    mode === "system" ? (systemDark ? "dark" : "light") : mode;

  // 同步 .dark class 到 <html>
  useEffect(() => {
    document.documentElement.classList.toggle("dark", resolved === "dark");
  }, [resolved]);

  // 同步 data-theme / data-art 到 <html> (首帧由 index.html 内联脚本先打好)
  useEffect(() => {
    applyThemeAttrs(themeId);
  }, [themeId]);

  // 监听系统明暗变化 (mode === "system" 时 resolved 随之重算)
  useEffect(() => {
    try {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      const handler = (e: MediaQueryListEvent) => setSystemDark(e.matches);
      mq.addEventListener("change", handler);
      return () => mq.removeEventListener("change", handler);
    } catch {
      return;
    }
  }, []);

  return (
    <ThemeContext.Provider
      value={{ mode, resolved, setMode, themeId, art: themeDef(themeId).art, setThemeId }}
    >
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme() {
  return useContext(ThemeContext);
}
