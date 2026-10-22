/**
 * 后端固定文案的本地化。
 *
 * 后端的 last_error_message / 测试连接 message 是中文原文 (TUI 原样显示, 也落 DB),
 * 同时另带一份结构化的 code。这里按 code 取 i18n 文案; code 缺失 (上游原文、老版本数据)
 * 时退回原文 —— 所以调用方总能拿到一段可显示的字符串。
 */
import type { TFunction } from "@/i18n";
import type { LastErrorCode, TestConnectionResult } from "@/types";

export function lastErrorText(
  code: LastErrorCode | null | undefined,
  raw: string,
  t: TFunction,
): string {
  if (!code) return raw;
  switch (code.code) {
    case "auth_failed":
      return t("lastError.authFailed", { status: code.status });
    case "rate_limited":
      return t("lastError.rateLimited");
    case "server_error":
      return t("lastError.serverError", { status: code.status });
    case "network":
      return t("lastError.network");
    case "upstream_quota_exhausted":
      return t("lastError.upstreamQuotaExhausted");
    case "upstream_rate_limited":
      return t("lastError.upstreamRateLimited");
  }
}

export function testResultText(r: TestConnectionResult, t: TFunction): string {
  const note = r.note;
  if (!note) return r.message;
  switch (note.code) {
    case "ok":
      return r.http_status != null
        ? t("testConnection.okWithStatus", { status: r.http_status })
        : t("testConnection.ok");
    case "network":
      return t("testConnection.network", { detail: note.detail });
    case "no_test_model":
      return t("testConnection.noTestModel");
  }
}
