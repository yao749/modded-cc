import { useEffect, useRef, useState } from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { CircleCheck, ExternalLink } from "lucide-react";
import { useT } from "@/i18n";
import { runtime } from "@/runtime";
import { ReceiptSlip, type ReceiptDisplayOptions } from "./ReceiptSlip";
import type { ReceiptDto } from "@/types";
import logoUrl from "@/assets/logo.png";

/**
 * 出票动画的全部时长。JS 计时器与 CSS 动画都读这一份 (经 --rp-* 变量传给样式表),
 * 改节奏只改这里。drop = 机器落位, feed = 走纸 (逐行显影), tear = 撕下。
 */
const PRINT_TIMING = { dropMs: 260, feedMs: 2000, tearMs: 320 } as const;
/** 落位后留一点余量再开始走纸, 避免落位与走纸的首档挤在同一帧 */
const FEED_START_MS = PRINT_TIMING.dropMs + 40;

type Phase = "idle" | "printing" | "done";

export type PrintExportStatus = "pending" | "ok";

interface Props {
  open: boolean;
  dto: ReceiptDto;
  options: ReceiptDisplayOptions;
  /** 正在导出的格式, 只用于状态文案 */
  kind: "png" | "pdf" | "html";
  /** 导出本身的进度; 动画放完且导出完成才显示「已保存」 */
  exportStatus: PrintExportStatus;
  onClose: () => void;
}

function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
  );
}

/**
 * 导出小票时的「小票机吐纸」动画 (设计稿: 小票打印动画 artifact)。
 * 纯装饰层: 导出在调用方照常进行, 这里只渲染一份同数据的小票副本做动画,
 * 随时可跳过 / Esc / 点遮罩关闭, 关掉不影响导出。
 */
export function ReceiptPrintOverlay({ open, dto, options, kind, exportStatus, onClose }: Props) {
  const { t } = useT();
  const [phase, setPhase] = useState<Phase>("idle");
  const timers = useRef<number[]>([]);

  useEffect(() => {
    const clear = () => {
      timers.current.forEach((id) => window.clearTimeout(id));
      timers.current = [];
    };
    if (!open) return clear;
    if (prefersReducedMotion()) {
      setPhase("done");
      return clear;
    }
    setPhase("idle");
    timers.current.push(window.setTimeout(() => setPhase("printing"), FEED_START_MS));
    timers.current.push(
      window.setTimeout(() => setPhase("done"), FEED_START_MS + PRINT_TIMING.feedMs),
    );
    return clear;
  }, [open]);

  const skip = () => {
    timers.current.forEach((id) => window.clearTimeout(id));
    timers.current = [];
    setPhase("done");
  };

  const saved = phase === "done" && exportStatus === "ok";

  const timingVars = {
    "--rp-drop": `${PRINT_TIMING.dropMs}ms`,
    "--rp-feed": `${PRINT_TIMING.feedMs}ms`,
    "--rp-tear": `${PRINT_TIMING.tearMs}ms`,
  } as React.CSSProperties;

  return (
    <DialogPrimitive.Root open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="cc-dialog-overlay rp-overlay" />
        <DialogPrimitive.Content className="rp" style={timingVars} aria-describedby={undefined}>
          <DialogPrimitive.Title className="sr-only">{t("receipts.print.title")}</DialogPrimitive.Title>

          <div className="rp-stage">
            <div className="rp-machine-pos">
              <div className={phase === "printing" ? "rp-buzz" : undefined}>
                <PrinterMachine printing={phase !== "done"} />
              </div>
            </div>

            <div className={"rp-window" + (phase === "done" ? " torn" : "")}>
              <div className="rp-shadow">
                <div className={"rp-paper " + phase}>
                  <div className="rp-edges">
                    <ReceiptSlip dto={dto} options={options} />
                  </div>
                </div>
              </div>
              <div className="rp-slot-shade" aria-hidden />
            </div>
          </div>

          <div className="rp-bar" role="status">
            {phase !== "done" ? (
              <>
                <span className="rp-bar-text">
                  {t("receipts.print.printing", { kind: kind.toUpperCase() })}
                </span>
                <button type="button" className="btn sm" onClick={skip}>
                  {t("receipts.print.skip")}
                </button>
              </>
            ) : saved ? (
              <div className="rp-bar-done">
                <CircleCheck size={16} className="rp-ok" aria-hidden />
                <span className="rp-bar-text strong">{t("receipts.savedToDownloads")}</span>
                {runtime.kind === "desktop" && (
                  <button
                    type="button"
                    className="btn sm"
                    onClick={() => void runtime.openDownloadsDir().catch((err) => console.warn(err))}
                  >
                    <ExternalLink size={12} /> {t("receipts.openDownloads")}
                  </button>
                )}
                <DialogPrimitive.Close className="btn primary sm">
                  {t("receipts.print.done")}
                </DialogPrimitive.Close>
              </div>
            ) : (
              <span className="rp-bar-text">{t("receipts.print.saving")}</span>
            )}
          </div>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

/** 小票机插画。配色写死 (与小票本身一样不随主题变), 结构见 styles.css 的 .rp-m-* */
function PrinterMachine({ printing }: { printing: boolean }) {
  return (
    <div className="rp-machine" aria-hidden>
      <div className="rp-m-shadow" />
      <div className="rp-m-lid">
        <div className="rp-m-cover" />
        <div className="rp-m-notch" />
        <div className="rp-m-power" />
      </div>
      <div className="rp-m-face">
        <div className="rp-m-brand">
          <img src={logoUrl} alt="" />
          <span>CC-ROUTER</span>
        </div>
        <div className={"rp-m-status" + (printing ? " printing" : "")}>
          <span className="rp-m-led" />
          <span>{printing ? "PRINTING" : "READY"}</span>
        </div>
      </div>
      <div className="rp-m-mouth" />
      <div className="rp-m-slot" />
      <div className="rp-m-teeth" />
    </div>
  );
}
