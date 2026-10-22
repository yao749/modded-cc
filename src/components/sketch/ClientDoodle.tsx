import type { CSSProperties, ReactNode } from "react";

/**
 * 实时路由图客户端便签上的涂鸦小图标 (32×32 画板), 笔法同侧栏图标 (SidebarIcon):
 * 纸色填充 + 叠了抖动滤镜的墨线。刻意不带背后的色块 —— 便签本身已经是一块颜色。
 * 滤镜 #ccr-rough-icon 由根节点的 <SketchDefs /> 提供。
 */
export type ClientDoodleName = "terminal" | "braces" | "laptop" | "bubble";

const PAPER: CSSProperties = { fill: "var(--bg)" };
const DOT: CSSProperties = { fill: "var(--stroke)", stroke: "none" };

const DRAW: Record<ClientDoodleName, (thin: CSSProperties) => ReactNode> = {
  // 终端窗: 标题栏 + 提示符, 光标一笔陶土色
  terminal: () => (
    <>
      <path style={PAPER} d="M5 8 C 12 7.4, 20 7.6, 27 7.2 L 27.4 24.8 C 20 25.2, 12 24.9, 4.8 25.3 Z" />
      <path d="M5 11.8 L27.2 11.6" />
      <path d="M8.6 15.6 L11.8 18 L8.4 20.4" />
      <path style={{ stroke: "var(--accent)" }} d="M13.8 20.6 L18.4 20.4" />
    </>
  ),
  // 一对花括号, 中间一颗陶土色的点
  braces: () => (
    <>
      <path d="M12 7.5 C 9 7.5, 9.6 11, 9.4 13.6 C 9.3 15.2, 8.2 15.8, 6.8 16 C 8.2 16.2, 9.3 16.8, 9.4 18.4 C 9.6 21, 9 24.5, 12 24.5" />
      <path d="M20 7.5 C 23 7.5, 22.4 11, 22.6 13.6 C 22.7 15.2, 23.8 15.8, 25.2 16 C 23.8 16.2, 22.7 16.8, 22.6 18.4 C 22.4 21, 23 24.5, 20 24.5" />
      <circle style={{ fill: "var(--accent)", stroke: "none" }} cx="16" cy="16" r="1.8" />
    </>
  ),
  // 笔记本电脑
  laptop: (thin) => (
    <>
      <path style={PAPER} d="M7.5 8 C 13 7.6, 19 7.8, 24.6 7.6 L 24.8 19.6 L 7.3 19.8 Z" />
      <path style={PAPER} d="M6 20 L 26 19.8 L 28 23.4 C 20 23.8, 12 23.2, 4 23.6 Z" />
      <path style={thin} d="M11 12.4 L15 12.4 M11 15.4 L19.6 15.2" />
    </>
  ),
  // 对话气泡里三个点 = 「其他」
  bubble: () => (
    <>
      <path style={PAPER} d="M6 9 C 12 7.4, 22 7.4, 26.4 9.4 C 28 14, 27.6 18, 25.8 20.6 C 21 22, 15 22, 12.4 21.4 L 8 25.6 L 8.8 20.4 C 5.6 18.6, 4.8 13, 6 9 Z" />
      <circle style={DOT} cx="11.4" cy="14.8" r="1.3" />
      <circle style={DOT} cx="16" cy="14.8" r="1.3" />
      <circle style={DOT} cx="20.6" cy="14.8" r="1.3" />
    </>
  ),
};

export function ClientDoodle({ name, size = 30 }: { name: ClientDoodleName; size?: number }) {
  // 屏幕线宽 ~1.7px, 换算回 32 画板
  const sw = (1.7 * 32) / size;
  return (
    <svg viewBox="0 0 32 32" width={size} height={size} aria-hidden="true" focusable="false" style={{ overflow: "visible", flexShrink: 0 }}>
      <g className="sk-ink" filter="url(#ccr-rough-icon)" style={{ strokeWidth: sw }}>
        {DRAW[name]({ strokeWidth: sw * 0.72 })}
      </g>
    </svg>
  );
}
