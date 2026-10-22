/**
 * 把 catch 到的错误变成可显示的文字。
 *
 * 后端命令的拒绝值是 `{ code, message }` 对象 (Tauri IPC 与网页端 JSON 同形), 不是 Error,
 * 直接 `String(e)` / `${e}` 会得到 "[object Object]"。插件与浏览器 API 抛的则是 Error 或字符串。
 */
export function errorText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return String(e);
}
