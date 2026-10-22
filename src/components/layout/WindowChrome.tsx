import { WindowControls } from "./WindowControls";
import { PixelIcon } from "@/components/win2k/PixelIcon";

/**
 * 叠在窗口顶部的透明拖窗带, 高度 = 系统标题栏高度 (CSS 变量 --titlebar-inset:
 * macOS 28px 对齐原生红绿灯, Windows / Linux 32px 对齐自绘三键)。
 * 它不占布局空间: 右侧内容从窗口顶边直接铺开, 只有侧栏为红绿灯留出这段高度。
 * Windows / Linux 上三键挂在带的右端; macOS 由系统画红绿灯, WindowControls 渲染为空。
 * 标题 (.window-title) 平时隐藏, 只有 Win2000 画风在 Windows / Linux 上把拖窗带画成渐变标题栏时显示。
 */
export function WindowChrome() {
  return (
    <div className="window-chrome" data-tauri-drag-region>
      <span className="window-title" data-tauri-drag-region aria-hidden="true">
        <PixelIcon name="live" />
        cc-router
      </span>
      <WindowControls />
    </div>
  );
}
