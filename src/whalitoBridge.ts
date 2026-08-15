// 与内嵌 DSH 页面"鲸仔"设置分区之间的 postMessage 桥。
// 下行（本窗口 → iframe）：hello / settings / status / error 快照；
// 上行（iframe → 本窗口）：ping / action。双方各自校验消息来源。

export interface WhalitoSettings {
  port: number;
  registry: string;
  autostart: boolean;
  autoRestart: boolean;
  workspaceDir: string | null;
  nodeDir: string | null;
  petEnabled: boolean;
}

export interface WhalitoStatus {
  phase: string;
  url: string | null;
  pid: number | null;
}

/** DSH 版本行（当前/最新/是否有更新）。 */
export interface DshVersionInfo {
  current: string | null;
  latest: string | null;
  updateAvailable: boolean;
}

/** 鲸仔版本行（附测试标记与下载页地址）。 */
export interface WhalitoVersionInfo {
  current: string | null;
  testBuild: boolean;
  latest: string | null;
  updateAvailable: boolean;
  url: string | null;
}

export interface VersionsSnapshot {
  dsh: DshVersionInfo;
  whalito: WhalitoVersionInfo;
}

export interface WhalitoMessage {
  channel: "whalito";
  type: string;
  action?: string;
  value?: unknown;
  settings?: WhalitoSettings | null;
  status?: WhalitoStatus;
  message?: string;
  versions?: VersionsSnapshot;
  target?: string;
  url?: unknown;
}

/** 当前 DSH 服务源（用于校验 iframe 消息的 event.origin）。 */
export function dshOrigin(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    return new URL(url).origin;
  } catch {
    return null;
  }
}

/**
 * 深度去代理/去响应式：Vue 的 reactive Proxy 无法通过 postMessage 的
 * structured clone（实测抛 DataCloneError），发送前必须 JSON 深拷贝成普通对象。
 */
export function toPlain<T>(value: T): T {
  if (value === null || value === undefined) return value;
  return JSON.parse(JSON.stringify(value)) as T;
}

/** 向 iframe 内的"鲸仔"设置分区发消息；返回 null 表示发送成功，否则返回错误信息。 */
export function postToDsh(
  frame: HTMLIFrameElement | null | undefined,
  msg: WhalitoMessage,
): string | null {
  if (!frame || !frame.contentWindow) {
    return "iframe 引用为空";
  }
  try {
    frame.contentWindow.postMessage(msg, "*");
    return null;
  } catch (e) {
    return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  }
}

/** 类型收窄：是否为鲸仔桥消息。 */
export function isWhalitoMessage(data: unknown): data is WhalitoMessage {
  if (typeof data !== "object" || data === null) return false;
  const d = data as Record<string, unknown>;
  return d.channel === "whalito" && typeof d.type === "string";
}
