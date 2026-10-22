/**
 * 全局共用的手绘抖动滤镜, 在根节点渲染一次, 插画里用 filter="url(#ccr-rough-*)" 引用。
 * feTurbulence 生成噪声, feDisplacementMap 用噪声把线条推歪: scale 越大越潦草。
 * primitiveUnits 默认是引用者的用户坐标, 所以两个滤镜按各自画板的尺度分别调参:
 * - ccr-rough-icon: 32 画板的侧栏图标
 * - ccr-rough-logo: 310 画板的 Logo
 * - ccr-rough-canvas: 实时路由图 (960 画布) 的连线、便签、吊牌 —— 频率低, 抖成长波浪而不是毛边
 * 滤镜区域外扩 15%, 不要挂在只含一条水平/竖直直线的元素上 (包围盒一边为 0, 线会消失)。
 */
export function SketchDefs() {
  return (
    <svg width="0" height="0" style={{ position: "absolute" }} aria-hidden="true" focusable="false">
      <defs>
        <filter id="ccr-rough-icon" x="-15%" y="-15%" width="130%" height="130%">
          <feTurbulence type="fractalNoise" baseFrequency="0.11" numOctaves={2} seed={7} result="n" />
          <feDisplacementMap in="SourceGraphic" in2="n" scale="0.9" xChannelSelector="R" yChannelSelector="G" />
        </filter>
        <filter id="ccr-rough-logo" x="-15%" y="-15%" width="130%" height="130%">
          <feTurbulence type="fractalNoise" baseFrequency="0.05" numOctaves={2} seed={7} result="n" />
          <feDisplacementMap in="SourceGraphic" in2="n" scale="1.8" xChannelSelector="R" yChannelSelector="G" />
        </filter>
        <filter id="ccr-rough-logo-sm" x="-15%" y="-15%" width="130%" height="130%">
          <feTurbulence type="fractalNoise" baseFrequency="0.05" numOctaves={2} seed={7} result="n" />
          <feDisplacementMap in="SourceGraphic" in2="n" scale="1" xChannelSelector="R" yChannelSelector="G" />
        </filter>
        <filter id="ccr-rough-canvas" x="-5%" y="-5%" width="110%" height="110%">
          <feTurbulence type="fractalNoise" baseFrequency="0.03" numOctaves={2} seed={7} result="n" />
          <feDisplacementMap in="SourceGraphic" in2="n" scale="2" xChannelSelector="R" yChannelSelector="G" />
        </filter>
      </defs>
    </svg>
  );
}
