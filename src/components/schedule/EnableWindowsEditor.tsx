import { useState } from "react";
import { Plus, X } from "lucide-react";
import { useT } from "@/i18n";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  crossesMidnight,
  dayPreset,
  DAY_PRESETS,
  hhmm,
  type DayPreset,
} from "@/lib/slotSchedule";
import type { EnableWindow } from "@/types";

const START_TIMES = Array.from({ length: 48 }, (_, i) => hhmm(i * 30));
const END_TIMES = Array.from({ length: 48 }, (_, i) => hhmm((i + 1) * 30));
const PRESET_ORDER: DayPreset[] = ["every", "workdays", "weekend", "custom"];
const WEEKDAYS = [1, 2, 3, 4, 5, 6, 7];

/**
 * 订阅级定时开关编辑器 (能力池订阅用): 有序的「开启窗口」列表, 每个窗口只有 星期 + 起止时间
 * (没有模型 / 思考强度)。窗口外订阅不参与调度。复用 SlotScheduleEditor 的 .sched-ed-* 样式和
 * 星期/时间选择逻辑 (`slotSchedule` 库)。
 */
export function EnableWindowsEditor({
  windows,
  onChange,
}: {
  windows: EnableWindow[];
  onChange: (w: EnableWindow[]) => void;
}) {
  const { t } = useT();
  const [customRows, setCustomRows] = useState<Set<number>>(new Set());

  const set = (next: EnableWindow[]) => {
    setCustomRows(new Set());
    onChange(next);
  };
  const edit = (i: number, patch: Partial<EnableWindow>) =>
    set(windows.map((w, j) => (j === i ? { ...w, ...patch } : w)));
  const remove = (i: number) => {
    setCustomRows(
      new Set([...customRows].filter((j) => j !== i).map((j) => (j > i ? j - 1 : j))),
    );
    set(windows.filter((_, j) => j !== i));
  };
  const add = () =>
    set([...windows, { days: [...DAY_PRESETS.every], start: "00:00", end: "08:00" }]);

  const setPreset = (i: number, p: DayPreset) => {
    const next = new Set(customRows);
    if (p === "custom") {
      next.add(i);
      setCustomRows(next);
      return;
    }
    next.delete(i);
    setCustomRows(next);
    edit(i, { days: [...DAY_PRESETS[p]] });
  };
  const toggleDay = (i: number, d: number) => {
    const days = windows[i].days;
    edit(i, {
      days: days.includes(d) ? days.filter((x) => x !== d) : [...days, d].sort((a, b) => a - b),
    });
  };

  return (
    <div className="sched-ed">
      {windows.length === 0 && (
        <div className="sched-ed-empty">{t("enableWindows.empty")}</div>
      )}
      {windows.map((w, i) => {
        const preset = customRows.has(i) ? "custom" : dayPreset(w.days);
        return (
          <div key={i} className="sched-ed-rule">
            <div className="sched-ed-rule-line">
              <span className="sched-ed-rule-no hand">
                {t("schedule.edit.ruleLabel", { n: i + 1 })}
              </span>
              <Select value={preset} onValueChange={(v) => setPreset(i, v as DayPreset)}>
                <SelectTrigger className="sched-ed-days">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {PRESET_ORDER.map((p) => (
                    <SelectItem key={p} value={p}>
                      {t(`schedule.edit.preset.${p}`)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Select value={w.start} onValueChange={(v) => edit(i, { start: v })}>
                <SelectTrigger className="sched-ed-time mono">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {START_TIMES.map((o) => (
                    <SelectItem key={o} value={o} className="font-mono">
                      {o}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <span className="sched-ed-sep" aria-hidden>
                –
              </span>
              <Select value={w.end} onValueChange={(v) => edit(i, { end: v })}>
                <SelectTrigger className="sched-ed-time mono">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {END_TIMES.map((o) => (
                    <SelectItem key={o} value={o} className="font-mono">
                      {o}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {crossesMidnight(w.start, w.end) && (
                <span className="pill warn">{t("schedule.edit.nextDay")}</span>
              )}
              <button
                type="button"
                className="sched-ed-del"
                onClick={() => remove(i)}
                aria-label={t("schedule.edit.removeAria", { n: i + 1 })}
                title={t("schedule.edit.removeAria", { n: i + 1 })}
              >
                <X size={14} />
              </button>
            </div>
            {preset === "custom" && (
              <div className="sched-ed-weekdays" role="group">
                {WEEKDAYS.map((d) => {
                  const on = w.days.includes(d);
                  return (
                    <button
                      key={d}
                      type="button"
                      className={on ? "on" : ""}
                      aria-pressed={on}
                      onClick={() => toggleDay(i, d)}
                    >
                      {t("schedule.day.one", { d: t(`schedule.wd.${d}`) })}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        );
      })}
      <div className="sched-ed-actions">
        <button type="button" className="btn sm" onClick={add}>
          <Plus size={13} /> {t("enableWindows.add")}
        </button>
      </div>
    </div>
  );
}