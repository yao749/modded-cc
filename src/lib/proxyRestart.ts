import type { ProxyMode, ProxyStatus } from "@/types";

/** window.location 里用到的部分, 便于传入固定值推理 */
export interface PageLocation {
  protocol: string;
  hostname: string;
  port: string;
  pathname: string;
  search: string;
  hash: string;
}

export interface DesiredProxy {
  proxy_mode: ProxyMode;
  proxy_port: number;
  https_port: number;
  listen_all: boolean;
}

const LOOPBACK = new Set(["localhost", "127.0.0.1", "[::1]"]);

type Scheme = "http" | "https";

function pageScheme(loc: PageLocation): Scheme {
  return loc.protocol === "https:" ? "https" : "http";
}

function pagePort(loc: PageLocation): number {
  if (loc.port) return Number(loc.port);
  return pageScheme(loc) === "https" ? 443 : 80;
}

/** 首选端口 → 实际端口, 只列出发生了顺延的那几路 */
export function shiftedPorts(s: ProxyStatus): Array<[number, number]> {
  const a = s.applied;
  if (!a) return [];
  const out: Array<[number, number]> = [];
  if (s.http_port != null && s.http_port !== a.proxy_port) out.push([a.proxy_port, s.http_port]);
  if (s.https_port != null && s.https_port !== a.https_port) out.push([a.https_port, s.https_port]);
  return out;
}

/**
 * 网页界面点重启前: 按已保存的设置预判本页地址会不会失效. 返回要确认的文案 key, 不失效返回 null.
 * 端口只能按首选端口预判 (是否顺延要绑了才知道), 顺延的情形由重启后的跳转兜住.
 */
export function webRestartWarning(loc: PageLocation, desired: DesiredProxy): string | null {
  if (!desired.listen_all && !LOOPBACK.has(loc.hostname)) {
    return "settings.proxy.restart.webLockout";
  }
  const scheme = pageScheme(loc);
  const enabled = scheme === "https" ? desired.proxy_mode !== "http" : desired.proxy_mode !== "https";
  const port = scheme === "https" ? desired.https_port : desired.proxy_port;
  if (!enabled || port !== pagePort(loc)) return "settings.proxy.restart.webMove";
  return null;
}

/**
 * 重启成功后本页的新地址: 保持 hostname 与 hash 路由, 端口换成与当前协议对应的实际端口;
 * 该协议已关闭则换到另一协议. 不需要跳转时返回 null.
 */
export function webUrlAfterRestart(loc: PageLocation, status: ProxyStatus): string | null {
  const scheme = pageScheme(loc);
  const samePort = scheme === "https" ? status.https_port : status.http_port;
  let target: { scheme: Scheme; port: number } | null = null;
  if (samePort != null) target = { scheme, port: samePort };
  else if (status.http_port != null) target = { scheme: "http", port: status.http_port };
  else if (status.https_port != null) target = { scheme: "https", port: status.https_port };
  if (!target) return null;
  if (target.scheme === scheme && target.port === pagePort(loc)) return null;
  return `${target.scheme}://${loc.hostname}:${target.port}${loc.pathname}${loc.search}${loc.hash}`;
}
