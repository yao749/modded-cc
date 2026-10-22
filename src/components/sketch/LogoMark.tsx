import type { CSSProperties } from "react";
import { useTheme } from "@/hooks/useTheme";
import { isPlainBased } from "@/themes";
import logoClassicUrl from "@/assets/logo-classic.png";

/**
 * cc-router 的手绘 Logo: 照原像素版一格一格描下来 (310 画板, 一格 = 10 单位) ——
 * 方身子、右上角两级台阶的耳朵、两只方眼睛、粉鼻头、四条腿, 头顶一颗带灯座的灯泡。
 * 画法同侧栏图标: 色块整体错开几个单位, 墨线叠抖动滤镜 (#ccr-rough-logo*, 见 SketchDefs)。
 *
 * - full: 大尺寸 (≥96px), 带灯丝、光芒、地面线
 * - compact: 小尺寸 (侧栏、favicon), 去掉细节, 取景收紧, 线条按像素加粗
 * - mono: 只留墨线 (托盘 / 单色场景)
 * - tile: 画在 App 图标式的圆角底板上
 *
 * 经典 / Win2000 画风下不画手绘版, 直接显示原来的像素 Logo, 尺寸与 tile 照旧。
 */
interface Props {
  size: number;
  /** 经典画风下的尺寸, 缺省同 size (像素 Logo 在改版前各处的尺寸与手绘版不同) */
  plainSize?: number;
  variant?: "full" | "compact";
  tile?: boolean;
  mono?: boolean;
  className?: string;
  /** 默认作为装饰隐藏; 独立出现 (无旁白文字) 时传入可读名称 */
  label?: string;
}

const BODY =
  "M91 151 C 120 149, 150 152, 170 150 L 171 141 C 181 140, 192 141, 200 140 L 199 101 C 206 100, 214 101, 220 100 L 221 110 C 228 110, 234 111, 240 110 C 241 160, 239 210, 240 259 C 190 261, 140 258, 90 260 C 89 220, 91 185, 91 151 Z";
const SNOUT = "M62 190 C 72 189, 82 191, 92 190 L 91 220 C 81 221, 70 219, 61 220 C 60 210, 61 200, 62 190 Z";
const BULB =
  "M155 12 C 181 11, 199 28, 198 50 C 197 66, 185 76, 182 90 L 130 90 C 127 76, 113 66, 112 50 C 111 28, 129 12, 155 12 Z";
const LEGS_OUTER = "M91 258 L90 299 L110 300 L110 259 Z M221 258 L220 299 L240 300 L240 259 Z";
const LEGS_INNER = "M131 258 L130 299 L150 300 L150 259 Z M191 258 L190 299 L210 300 L210 259 Z";

export function LogoMark(props: Props) {
  const { art } = useTheme();
  return isPlainBased(art) ? <PixelLogo {...props} /> : <SketchLogo {...props} />;
}

function PixelLogo({ size, plainSize = size, tile = false, className, label }: Props) {
  const cls = ["logo-plain", tile && "tile", className].filter(Boolean).join(" ");
  return (
    <span className={cls} style={{ width: plainSize, height: plainSize }}>
      <img src={logoClassicUrl} alt={label ?? ""} aria-hidden={label ? undefined : true} />
    </span>
  );
}

function SketchLogo({ size, variant = "full", tile = false, mono = false, className, label }: Props) {
  const compact = variant === "compact";
  // 线宽按屏幕像素定, 再换算回画板单位: 大图约 0.9% 边长, 侧栏 1.3px, 16px 时 1px
  const screen = size >= 200 ? size * 0.0092 : size >= 96 ? 1.4 : size >= 40 ? 1.3 : 1;
  const sw = (screen * (compact ? 262 : 330)) / size;
  const thin: CSSProperties = { strokeWidth: sw * 0.72 };
  const fillOpacity = mono ? 0 : 1;
  const fill = (color: string): CSSProperties => ({ fill: color, fillOpacity });
  const filter = compact ? "url(#ccr-rough-logo-sm)" : "url(#ccr-rough-logo)";

  const svg = (
    <svg
      viewBox={compact ? "46 6 262 300" : "-10 -20 330 330"}
      width="100%"
      height="100%"
      role={label && !tile ? "img" : undefined}
      aria-label={label && !tile ? label : undefined}
      aria-hidden={label && !tile ? undefined : true}
      focusable="false"
      style={{ overflow: "visible", display: "block" }}
    >
      {tile && !mono && !compact && (
        <path
          style={{ fill: "var(--blob)" }}
          d="M40 120 C 50 50, 170 20, 250 60 C 300 90, 302 200, 262 258 C 222 302, 100 306, 60 270 C 25 240, 32 170, 40 120 Z"
        />
      )}
      <path transform="translate(4 3)" style={fill("var(--accent)")} d={LEGS_OUTER} />
      <path transform="translate(4 3)" style={fill("var(--fill-clay-deep)")} d={LEGS_INNER} />
      <path transform="translate(6 5)" style={fill("var(--accent)")} d={BODY} />
      <path transform="translate(4 3)" style={fill("var(--accent)")} d={SNOUT} />
      <path transform="translate(3 2)" style={fill("var(--fill-nose)")} d="M61 190 L73 190 L72 207 L61 208 Z" />
      <g className="sk-ink" filter={filter} style={{ strokeWidth: sw }}>
        <path d="M91 258 L90 299 L110 300 L110 259 M131 258 L130 299 L150 300 L150 259 M191 258 L190 299 L210 300 L210 259 M221 258 L220 299 L240 300 L240 259" />
        <path d={BODY} />
        <path d="M91 190 C 81 191, 72 189, 62 190 C 61 200, 60 210, 61 220 C 70 219, 81 221, 91 220" />
        {!compact && <path style={thin} d="M73 190.5 L72.5 207.5 L61 208" />}
        {!compact && <path style={thin} d="M60 303 C 120 299, 190 305, 262 301" />}
      </g>
      <path
        style={{ fill: "var(--stroke)" }}
        d="M111 171 C 117 170, 124 171, 129 170 L 130 189 C 124 190, 117 189, 110 189 Z M201 161 C 207 160, 214 161, 219 160 L 220 179 C 214 180, 207 179, 200 179 Z"
      />
      <path transform="translate(5 4)" style={fill("var(--fill-butter)")} d={BULB} />
      <path transform="translate(4 3)" style={fill("var(--fill-stone)")} d="M132 92 L180 92 L179 102 L174 110 L141 110 L135 102 Z" />
      <g className="sk-ink" filter={filter} style={{ strokeWidth: sw }}>
        <path d={BULB} />
        <path d="M131 92 C 148 91, 164 93, 181 92 M136 101.5 C 150 101, 164 102, 178 101.5 M142 110 C 152 109.5, 162 110.5, 172 110" />
        {!compact && (
          <>
            <path style={{ ...thin, stroke: "var(--accent-ink)" }} d="M139 52 C 141 62, 143 71, 149 78 M171 52 C 169 62, 167 71, 161 78" />
            <path style={thin} d="M155 -2 L155 -12 M118 10 L110 2 M192 10 L200 2 M100 46 L89 44 M210 46 L221 44" />
          </>
        )}
      </g>
    </svg>
  );

  if (!tile) {
    return (
      <span className={className} style={{ display: "inline-block", width: size, height: size, flexShrink: 0, lineHeight: 0 }}>
        {svg}
      </span>
    );
  }
  return (
    <span
      className={className ? `logo-tile ${className}` : "logo-tile"}
      role={label ? "img" : undefined}
      aria-label={label}
      style={{ width: size, height: size, padding: Math.round(size * (compact ? 0.12 : 0.05)) }}
    >
      {svg}
    </span>
  );
}
