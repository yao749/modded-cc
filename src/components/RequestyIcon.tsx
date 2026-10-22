import { memo, useId } from "react";

// Requesty 不在 @lobehub/icons 里 (核对 5.21 / 2026-09), 按官方 logo 手绘简化版:
// 蓝色对话气泡 + 白色 ">_"。形状与 lobehub 图标同一套接口 (默认导出 = 单色, .Color = 彩色),
// 好直接登记进 ProviderIcon 的 BRAND_MAP。

const BRAND_BLUE = "#1677FF";
const BUBBLE =
  "M4 2.5h16A2.5 2.5 0 0 1 22.5 5v10a2.5 2.5 0 0 1-2.5 2.5h-9.2L8.2 22l-.8-4.5H4A2.5 2.5 0 0 1 1.5 15V5A2.5 2.5 0 0 1 4 2.5z";
const PROMPT = "M5.8 6.4 9.9 9.6 5.8 12.8";

interface IconProps {
  size?: number | string;
}

function Glyph({ stroke }: { stroke: string }) {
  return (
    <>
      <path d={PROMPT} fill="none" stroke={stroke} strokeWidth={1.9} strokeLinecap="square" />
      <rect x={11.4} y={12.6} width={5.4} height={1.9} fill={stroke} />
    </>
  );
}

const Color = memo(function RequestyColor({ size = "1em" }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" style={{ flex: "none", lineHeight: 1 }}>
      <title>Requesty</title>
      <path d={BUBBLE} fill={BRAND_BLUE} />
      <Glyph stroke="#fff" />
    </svg>
  );
});

// 单色: 气泡用 currentColor, ">_" 用 mask 镂空, 深浅底都成立
const Mono = memo(function RequestyMono({ size = "1em" }: IconProps) {
  const maskId = useId();
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" style={{ flex: "none", lineHeight: 1 }}>
      <title>Requesty</title>
      <mask id={maskId}>
        <rect width={24} height={24} fill="#fff" />
        <Glyph stroke="#000" />
      </mask>
      <path d={BUBBLE} fill="currentColor" mask={`url(#${maskId})`} />
    </svg>
  );
});

const Requesty = Object.assign(Mono, { Color, colorPrimary: BRAND_BLUE });

export default Requesty;
