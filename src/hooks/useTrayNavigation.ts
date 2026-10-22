import { useEffect } from "react";
import { useNavigate } from "react-router";
import { runtime } from "@/runtime";

/** 与 Rust `tray::NAVIGATE_EVENT` 一致 */
const TRAY_NAVIGATE_EVENT = "tray://navigate";

/**
 * 托盘菜单的页面快捷方式 (实时路由 / 请求日志 / 订阅管理 / 检查更新): Rust 呼出窗口后
 * 发来目标路径, 这里切过去。挂在 App 顶层而不是 AppShell —— 后者在引导期间不渲染。
 * 只有桌面端会收到这个事件 (后端只发给主窗口、不桥接给网页界面), 网页端直接不监听。
 */
export function useTrayNavigation() {
  const navigate = useNavigate();
  useEffect(() => {
    if (runtime.kind !== "desktop") return;
    const promise = runtime.listen<string>(TRAY_NAVIGATE_EVENT, (e) => {
      if (typeof e.payload === "string" && e.payload.startsWith("/")) navigate(e.payload);
    });
    return () => {
      promise.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [navigate]);
}
