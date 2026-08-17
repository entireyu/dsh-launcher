<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { dshOrigin, isWhalitoMessage, postToDsh, toPlain } from "./whalitoBridge";
import type {
  VersionsSnapshot,
  WhalitoMessage,
  WhalitoSettings,
  WhalitoVersionInfo,
} from "./whalitoBridge";

interface EnvInfo {
  found: boolean;
  version: string | null;
  nodePath: string | null;
  npmPrefix: string | null;
  installPrefix: string | null;
  dshInstalled: boolean;
  dshVersion: string | null;
  nodeVersionOk: boolean;
  nodeTooOld: boolean;
  nvmFound: boolean;
  nvmPath: string | null;
}

interface ServerStatus {
  phase: string;
  url: string | null;
  pid: number | null;
}

interface Settings {
  port: number;
  registry: string;
  /** DSH 版本偏好："latest"（稳定版）/ "next"（预发布版）。 */
  dshChannel: string;
  autostart: boolean;
  autoRestart: boolean;
  workspaceDir: string | null;
  nodeDir: string | null;
  downloadDir: string | null;
  petEnabled: boolean;
}

const env = ref<EnvInfo | null>(null);
const server = ref<ServerStatus>({ phase: "stopped", url: null, pid: null });
const settings = ref<Settings | null>(null);
// 当前平台（"windows" / "macos" / "linux"），由后端 get_platform 命令返回。
const platform = ref<string>("windows");
const logs = ref<string[]>([]);
const busy = ref<string | null>(null);
const error = ref<string>("");
const notice = ref<string>("");
const showSettings = ref(false);
const confirmingStop = ref(false);
const autoRestartCount = ref(0);
const MAX_AUTO_RESTART = 3;

// 内嵌 DSH 页面右键自定义菜单（复制/剪切/粘贴 ─ 刷新页面 / 重启服务器 / 显示隐藏桌宠）的位置；null = 关闭。
const ctxMenu = ref<{ x: number; y: number } | null>(null);

// 下载完成提示（会话日志导出等）：保存路径 + 自动消失计时器。
const toast = ref<{ text: string; path: string } | null>(null);
let toastTimer: number | undefined;

const installingNode = ref(false);
const installingDsh = ref(false);
const verifying = ref(false);
const checkingUpdate = ref(false);
const latestVersion = ref<string | null>(null);

// 视图：flow = 引导流程 / panel = 高级面板 / embed = 内嵌页面
const view = ref<"flow" | "panel" | "embed">("flow");
const stage = ref<"detecting" | "node" | "dsh" | "server">("detecting");
const flowError = ref<string>("");
const installStage = ref<string>("");

const embedNonce = ref(0);
const embedFrame = ref<HTMLIFrameElement | null>(null);
const whalitoVer = ref<WhalitoVersionInfo | null>(null);
let whalitoPingLogged = false;

const stageText: Record<string, string> = {
  install: "正在安装…",
  use: "正在切换版本…",
  download: "正在下载…",
  extract: "正在解压…",
  verify: "正在校验…",
  error: "安装失败",
};

const updateAvailable = computed(() => {
  if (!env.value?.dshInstalled || !env.value.dshVersion || !latestVersion.value) return false;
  return latestVersion.value !== env.value.dshVersion;
});

const isLatest = computed(() => {
  return (
    !!env.value?.dshInstalled &&
    !!latestVersion.value &&
    latestVersion.value === env.value.dshVersion
  );
});

const unlisteners: UnlistenFn[] = [];
let pollTimer: number | undefined;
let versionTimer: number | undefined;
let lastTrayRunning: boolean | null = null;

const phaseText: Record<string, string> = {
  stopped: "已停止",
  starting: "启动中",
  running: "运行中",
  external: "运行中（外部）",
  error: "异常",
};

const phaseClass = computed(() => server.value.phase);
const serverPhaseText = computed(() => phaseText[server.value.phase] ?? server.value.phase);

async function wrap<T>(task: string, fn: () => Promise<T>): Promise<T | undefined> {
  busy.value = task;
  error.value = "";
  notice.value = "";
  try {
    return await fn();
  } catch (e) {
    error.value = typeof e === "string" ? e : String(e);
    return undefined;
  } finally {
    busy.value = null;
  }
}

async function refreshEnv() {
  try {
    env.value = await invoke<EnvInfo>("detect_env");
  } catch {
    env.value = null;
  }
}

async function refreshStatus() {
  try {
    server.value = await invoke<ServerStatus>("server_status");
    if (server.value.phase !== "external") confirmingStop.value = false;
    if (server.value.phase === "running") autoRestartCount.value = 0;
    syncTray();
  } catch {
    /* 忽略瞬时错误 */
  }
}

function syncTray() {
  const running = server.value.phase !== "stopped";
  if (running !== lastTrayRunning) {
    lastTrayRunning = running;
    invoke("update_tray_state", { running }).catch(() => {});
  }
}

async function refreshAll() {
  await Promise.all([refreshEnv(), refreshStatus()]);
}

async function installNode() {
  installingNode.value = true;
  installStage.value = "install";
  try {
    const r = await wrap("正在安装 Node.js…", () => invoke<EnvInfo>("install_node"));
    if (r) {
      env.value = r;
      await runFlow();
    }
  } finally {
    installingNode.value = false;
  }
}

async function upgradeNode() {
  installingNode.value = true;
  installStage.value = "install";
  try {
    const r = await wrap("正在升级 Node.js…", () => invoke<EnvInfo>("upgrade_node"));
    if (r) {
      env.value = r;
      await runFlow();
    }
  } finally {
    installingNode.value = false;
  }
}

async function installNodeNvm() {
  installingNode.value = true;
  installStage.value = "install";
  try {
    const r = await wrap("正在通过 nvm 安装 Node.js…", () => invoke<EnvInfo>("install_node_nvm"));
    if (r) {
      env.value = r;
      await runFlow();
    }
  } finally {
    installingNode.value = false;
  }
}

async function installNodePortable() {
  installingNode.value = true;
  try {
    const dir = await invoke<string | null>("pick_node_dir");
    if (!dir) return;
    installStage.value = "download";
    const r = await wrap("正在下载并安装便携版 Node.js…", () =>
      invoke<EnvInfo>("install_node_portable", { dir }),
    );
    if (r) {
      env.value = r;
      settings.value = await invoke<Settings>("get_settings");
      await runFlow();
    }
  } finally {
    installingNode.value = false;
  }
}

async function installDsh() {
  installingDsh.value = true;
  installStage.value = "install";
  try {
    const r = await wrap("正在安装 DeepSeek Harness…", () => invoke<EnvInfo>("install_dsh"));
    if (r) env.value = r;
  } finally {
    installingDsh.value = false;
  }
}

async function updateDsh() {
  installingDsh.value = true;
  try {
    const r = await wrap("正在更新 DeepSeek Harness…", () =>
      invoke<EnvInfo>("update_dsh"),
    );
    if (r) {
      env.value = r;
      checkLatest();
    }
  } finally {
    installingDsh.value = false;
  }
}

async function verifyDsh() {
  verifying.value = true;
  try {
    const r = await wrap("正在校验…", () => invoke<string>("verify_dsh"));
    if (r) notice.value = r;
  } finally {
    verifying.value = false;
  }
}

async function startServer() {
  autoRestartCount.value = 0;
  const r = await wrap("正在启动…", () => invoke<ServerStatus>("start_server"));
  if (r) server.value = r;
}

async function doStop() {
  const r = await wrap("正在停止…", () => invoke<ServerStatus>("stop_server"));
  if (r) server.value = r;
}

async function restartServer() {
  confirmingStop.value = false;
  const r = await wrap("正在重启…", () => invoke<ServerStatus>("restart_server"));
  if (r) server.value = r;
}

// ============ 内嵌 DSH 页面右键菜单 ============
/** 刷新页面：重新挂载 iframe（key 变化触发重建，等同浏览器刷新 DSH 页面）。 */
function ctxReload() {
  ctxMenu.value = null;
  embedNonce.value += 1;
}

/** 重启服务器：与设置分区里的「重启服务器」动作一致。 */
async function ctxRestart() {
  ctxMenu.value = null;
  await restartServer();
  embedNonce.value += 1;
  pushSnapshot();
}

/** 显示/隐藏桌宠：与托盘菜单一致——由 Rust 按真实状态翻转，前端只同步结果。 */
async function ctxTogglePet() {
  ctxMenu.value = null;
  const r = await wrap("正在更新桌宠…", () => invoke<boolean>("pet_toggle"));
  if (r !== undefined) {
    if (settings.value) settings.value.petEnabled = r;
    pushSnapshot();
  }
}

// ============ 内嵌页面右键剪贴板操作 ============
// 选区/光标在内嵌 iframe（跨源），剪贴板由父窗口经 Rust 代为读写：
// 复制 → 通知 iframe 读选区文本上行 → 写入系统剪贴板；
// 粘贴 → 父窗口读剪贴板 → 下发文本 → iframe 在光标处插入（组件自己的粘贴管线）。
function ctxCopy() {
  ctxMenu.value = null;
  diag("menu copy clicked");
  postToDsh(embedFrame.value, { channel: "whalito", type: "action", action: "context-copy" });
}

async function ctxPaste() {
  ctxMenu.value = null;
  diag("menu paste clicked");
  try {
    const text = await invoke<string>("clipboard_read");
    diag(`clipboard_read ok, len=${text.length}`);
    postToDsh(embedFrame.value, {
      channel: "whalito",
      type: "action",
      action: "context-paste",
      text,
    });
  } catch (e) {
    const msg = typeof e === "string" ? e : String(e);
    diag(`clipboard_read failed: ${msg}`);
    postWhalitoError(`读取剪贴板失败：${msg}`);
  }
}

/** 鲸仔面板自身右键不弹任何菜单（自定义菜单只服务内嵌 DSH 页面）。 */
function onPanelContextMenu(e: MouseEvent) {
  e.preventDefault();
  ctxMenu.value = null;
}

// ============ 下载提示 ============
/** 弹提示，durationMs 后自动消失；重复提示重置计时。下载提示默认 8s，复制提示 2s。 */
function showToast(text: string, path: string, durationMs = 8000) {
  toast.value = { text, path };
  if (toastTimer !== undefined) window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toast.value = null;
  }, durationMs);
}

/** 「打开所在文件夹」：在系统文件管理器里定位下载文件。 */
async function revealDownload(path: string) {
  await invoke("reveal_in_folder", { path }).catch((e) => {
    notice.value = typeof e === "string" ? e : String(e);
  });
}

/** 面板设置里的目录选择（工作目录 / 下载目录）。 */
async function pickWorkspaceDir() {
  if (!settings.value) return;
  const dir = await invoke<string | null>("pick_directory");
  if (dir) settings.value.workspaceDir = dir;
}

async function pickDownloadDir() {
  if (!settings.value) return;
  const dir = await invoke<string | null>("pick_directory");
  if (dir) settings.value.downloadDir = dir;
}

async function stopServer() {
  if (server.value.phase === "external" && !confirmingStop.value) {
    confirmingStop.value = true;
    return;
  }
  confirmingStop.value = false;
  await doStop();
}

async function stopServerTray() {
  confirmingStop.value = false;
  await doStop();
}

async function openUrl() {
  if (server.value.url) {
    await invoke("open_url", { url: server.value.url });
  }
}

function openEmbedded() {
  view.value = "embed";
}

// ============ 与内嵌 DSH 页面"鲸仔"设置分区通信 ============
function onEmbedLoad() {
  pushSnapshot();
}

/** 组装版本快照：DSH 来自环境检测 + 最近一次检查结果；鲸仔来自 Rust 命令缓存。 */
function buildVersions(): VersionsSnapshot {
  const dshCurrent = env.value?.dshVersion ?? null;
  return {
    dsh: {
      current: dshCurrent,
      latest: latestVersion.value,
      updateAvailable:
        latestVersion.value !== null &&
        dshCurrent !== null &&
        latestVersion.value !== dshCurrent,
    },
    whalito: whalitoVer.value
      ? toPlain(whalitoVer.value)
      : {
          current: null,
          testBuild: false,
          latest: null,
          updateAvailable: false,
          autoUpdate: false,
          url: null,
        },
  };
}

function pushSnapshot() {
  // 注意：必须 toPlain 去响应式——Vue reactive Proxy 过不了 postMessage
  // 的 structured clone（会抛 DataCloneError）；settings 未加载时也回握手。
  const err = postToDsh(embedFrame.value, {
    channel: "whalito",
    type: "hello",
    settings: settings.value ? toPlain(settings.value) : null,
    status: toPlain(server.value),
    versions: toPlain(buildVersions()),
  });
  if (err !== null) {
    invoke("bridge_diag", { line: `推送快照失败：${err}` }).catch(() => {});
  }
}

function postWhalitoError(message: string) {
  postToDsh(embedFrame.value, { channel: "whalito", type: "error", message });
}

/** 剪贴板链路诊断：写入 %TEMP%\whalito-bridge.log。 */
function diag(line: string) {
  invoke("bridge_diag", { line: `[clip] ${line}` }).catch(() => {});
}

function isPortValid(p: unknown): p is number {
  return typeof p === "number" && Number.isInteger(p) && p >= 1 && p <= 65535;
}

// 内嵌页消息串行队列：保存设置 / 检查更新 / 更新安装共享 latestVersion 等状态，
// 并发交错会让旧结果覆盖新结果（如：切回稳定版后仍显示预发布版的新版本）。
let whalitoQueue: Promise<void> = Promise.resolve();
function enqueueWhalito<T>(fn: () => Promise<T>): Promise<T> {
  const run = whalitoQueue.then(fn, fn);
  whalitoQueue = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

async function handleWhalitoMessage(event: MessageEvent) {
  const origin = dshOrigin(server.value.url);
  if (origin !== null && event.origin !== origin) return;
  if (!isWhalitoMessage(event.data)) return;
  const msg = event.data;
  await enqueueWhalito(() => processWhalitoMessage(msg, event.origin));
}

async function processWhalitoMessage(msg: WhalitoMessage, eventOrigin: string) {
  if (msg.type === "ping") {
    if (!whalitoPingLogged) {
      whalitoPingLogged = true;
      logs.value.push("[系统] 鲸仔设置分区已连接（收到内嵌页握手请求）");
      invoke("bridge_diag", { line: `收到 ping，origin=${eventOrigin}` }).catch(() => {});
    }
    pushSnapshot();
    return;
  }
  if (msg.type !== "action") return;
  const action = msg.action;
  try {
    if (action === "save-settings") {
      const value = msg.value as WhalitoSettings | null;
      if (!value || !isPortValid(value.port)) {
        postWhalitoError("无效的端口（需要 1–65535 的整数）");
        return;
      }
      const prevPort = settings.value?.port;
      const r = await wrap("正在保存设置…", () =>
        invoke<Settings>("save_settings", { value }),
      );
      if (!r) {
        postWhalitoError(error.value || "保存设置失败");
        return;
      }
      settings.value = r;
      await wrap("正在更新开机自启…", () =>
        invoke<boolean>("set_autostart", { enabled: r.autostart }),
      );
      await wrap("正在更新桌宠…", () =>
        invoke<boolean>("pet_set_enabled", { enabled: r.petEnabled }),
      );
      settings.value = await invoke<Settings>("get_settings");
      // 保存后无条件重查版本：版本偏好/镜像源都会影响检查结果，
      // 无条件重查 + 串行队列保证最终显示与当前设置一致（不会残留旧通道结果）。
      latestVersion.value = null;
      await checkLatest();
      if (
        prevPort !== undefined &&
        r.port !== prevPort &&
        (server.value.phase === "running" || server.value.phase === "external")
      ) {
        notice.value = "端口已变更，正在重启服务器…";
        await restartServer();
        embedNonce.value += 1;
      }
      pushSnapshot();
      return;
    }
    if (action === "start") {
      await startServer();
      if (server.value.url) {
        view.value = "embed";
        embedNonce.value += 1;
      }
      pushSnapshot();
      return;
    }
    if (action === "stop") {
      await doStop();
      view.value = "panel";
      return;
    }
    if (action === "restart") {
      await restartServer();
      embedNonce.value += 1;
      pushSnapshot();
      return;
    }
    if (action === "focus-panel") {
      goPanel();
      return;
    }
    if (action === "check-update") {
      const target = msg.target;
      if (target === "dsh") {
        await checkLatest();
        if (latestVersion.value === null) {
          postWhalitoError("无法获取 DSH 最新版本（检查失败或已是最新）");
        }
      } else if (target === "whalito") {
        const r = await invoke<WhalitoVersionInfo>("whalito_check_update").catch((e) => {
          postWhalitoError(typeof e === "string" ? e : String(e));
          return null;
        });
        whalitoVer.value = r ?? whalitoVer.value;
      }
      pushSnapshot();
      return;
    }
    if (action === "update-dsh") {
      // 设置页「立即更新」：按当前版本偏好安装；进度经 install-stage → update-progress 回传。
      installingDsh.value = true;
      postToDsh(embedFrame.value, {
        channel: "whalito",
        type: "update-progress",
        message: "正在更新 DeepSeek Harness…",
      });
      try {
        const r = await invoke<EnvInfo>("update_dsh");
        env.value = r;
        await checkLatest();
        // 更新替换了 DSH 安装目录：重启服务器并重挂 iframe，让新版本立即生效
        // （与端口变更后的自动重启流程一致）。
        if (server.value.phase === "running" || server.value.phase === "external") {
          postToDsh(embedFrame.value, {
            channel: "whalito",
            type: "update-progress",
            message: "更新完成，正在重启服务器…",
          });
          await restartServer();
          embedNonce.value += 1;
        }
      } catch (e) {
        postWhalitoError(typeof e === "string" ? e : String(e));
      } finally {
        installingDsh.value = false;
        pushSnapshot();
      }
      return;
    }
    if (action === "open-url") {
      const url = msg.url;
      if (typeof url === "string" && url.startsWith("https://")) {
        await invoke("open_url", { url }).catch((e) => postWhalitoError(String(e)));
      } else {
        postWhalitoError("无效的下载地址");
      }
      return;
    }
    if (action === "apply-update") {
      // 命令末尾会退出应用（安装链接管重启），不 await；失败时回传错误。
      invoke("whalito_apply_update").catch((e) =>
        postWhalitoError(typeof e === "string" ? e : String(e)),
      );
      return;
    }
    if (action === "context-menu") {
      // 仅在内嵌 DSH 页面视图弹出（面板视图不显示右键菜单），并做边缘收拢。
      if (view.value === "embed" && typeof msg.x === "number" && typeof msg.y === "number") {
        const menuWidth = 170;
        const menuHeight = 250;
        const margin = 8;
        ctxMenu.value = {
          x: Math.max(margin, Math.min(msg.x, window.innerWidth - menuWidth - margin)),
          y: Math.max(margin, Math.min(msg.y, window.innerHeight - menuHeight - margin)),
        };
      }
      return;
    }
    if (action === "context-menu-close") {
      ctxMenu.value = null;
      return;
    }
    if (action === "clipboard-set") {
      // 内嵌页面复制：选区文本上行到这里 → 写入系统剪贴板。
      const text = msg.text ?? "";
      if (!text) return;
      diag(`clipboard-set received, len=${text.length}, dbg=${JSON.stringify(msg.dbg ?? null)}`);
      const ok = await invoke<void>("clipboard_write", { text })
        .then(() => true)
        .catch((e) => {
          diag(`clipboard_write failed: ${typeof e === "string" ? e : String(e)}`);
          return false;
        });
      if (ok) {
        diag("clipboard_write ok");
        showToast("已复制到剪贴板", "", 2000);
      } else {
        postWhalitoError("复制到剪贴板失败");
      }
      return;
    }
    if (action === "clipboard-noop") {
      diag(`clipboard-noop: ${typeof msg.why === "string" ? msg.why : "?"}`);
      return;
    }
    if (action === "pick-directory") {
      // DSH 设置分区请求原生目录选择：选完经 picked-dir 消息回填草稿。
      const dir = await invoke<string | null>("pick_directory").catch(() => null);
      if (dir && (msg.field === "workspaceDir" || msg.field === "downloadDir")) {
        postToDsh(embedFrame.value, {
          channel: "whalito",
          type: "picked-dir",
          field: msg.field,
          path: dir,
        });
      }
      return;
    }
    if (action === "whalito-download") {
      // 会话日志导出下载：由鲸仔下载到配置目录，完成弹提示。
      if (typeof msg.url === "string" && typeof msg.filename === "string") {
        const path = await invoke<string>("whalito_download", {
          url: msg.url,
          filename: msg.filename,
        }).catch((e) => {
          postWhalitoError(typeof e === "string" ? e : String(e));
          return null;
        });
        if (path !== null) showToast("会话日志已保存", path);
      }
      return;
    }
    postWhalitoError(`未知动作：${action ?? ""}`);
  } catch (e) {
    postWhalitoError(typeof e === "string" ? e : String(e));
  }
}

async function refreshLogs() {
  logs.value = await invoke<string[]>("get_logs");
}

function clearLogs() {
  logs.value = [];
}

async function loadSettings() {
  settings.value = await invoke<Settings>("get_settings");
  whalitoVer.value = await invoke<WhalitoVersionInfo>("whalito_version_info").catch(
    () => null,
  );
}

async function saveSettings() {
  if (!settings.value) return;
  const r = await wrap("正在保存设置…", () =>
    invoke<Settings>("save_settings", { value: settings.value }),
  );
  if (r) {
    settings.value = r;
    notice.value = "设置已保存";
    showSettings.value = false;
    pushSnapshot();
  }
}

async function toggleAutostart() {
  if (!settings.value) return;
  const enabled = settings.value.autostart;
  const r = await wrap("正在更新开机自启…", () =>
    invoke<boolean>("set_autostart", { enabled }),
  );
  if (r !== undefined && settings.value) {
    settings.value.autostart = r;
    pushSnapshot();
  }
}

async function togglePet() {
  if (!settings.value) return;
  const enabled = settings.value.petEnabled;
  const r = await wrap("正在更新桌宠…", () =>
    invoke<boolean>("pet_set_enabled", { enabled }),
  );
  if (r !== undefined && settings.value) {
    settings.value.petEnabled = r;
    pushSnapshot();
  }
}

async function checkLatest(showSpinner = false) {
  if (showSpinner) checkingUpdate.value = true;
  try {
    latestVersion.value = await invoke<string | null>("check_latest_version");
  } catch {
    /* 忽略 */
  } finally {
    if (showSpinner) checkingUpdate.value = false;
  }
}

/** 主流程编排：检测 → 装 Node / 装 dsh / 启动服务器 → 内嵌打开。 */
async function runFlow() {
  flowError.value = "";
  await refreshAll();

  const e = env.value;
  if (!e) {
    stage.value = "detecting";
    flowError.value = "环境检测失败，请重试。";
    return;
  }

  // 1. Node 缺失或版本过低 → 提示安装（等待用户选择）
  if (!e.found || e.nodeTooOld) {
    stage.value = "node";
    return;
  }

  // 2. Node 正常、未装 dsh → 自动安装
  if (!e.dshInstalled) {
    stage.value = "dsh";
    await installDsh();
    if (!env.value?.dshInstalled) {
      flowError.value = "DeepSeek Harness 安装未完成，请重试或查看日志。";
      return;
    }
  }

  // 3. 确保服务器运行
  stage.value = "server";
  await refreshStatus();
  if (server.value.phase !== "running" && server.value.phase !== "external") {
    await startServer();
    if (server.value.phase !== "running") {
      flowError.value = "服务器启动失败，请重试或进入高级面板查看日志。";
      return;
    }
  }

  // 4. 内嵌打开（同一窗口）
  if (server.value.url) {
    view.value = "embed";
  } else {
    flowError.value = "未能获取服务器地址，请重试。";
  }
}

function goPanel() {
  view.value = "panel";
}

onMounted(async () => {
  unlisteners.push(
    await listen<string>("log", (e) => {
      logs.value.push(e.payload);
      if (logs.value.length > 2000) logs.value.splice(0, logs.value.length - 2000);
      autoScroll();
    }),
  );
  unlisteners.push(
    await listen<string>("server-url", (e) => {
      server.value.url = e.payload;
    }),
  );
  unlisteners.push(
    await listen<string>("install-stage", (e) => {
      installStage.value = e.payload;
      // 安装阶段回传设置分区（update-dsh / 面板安装都可见进度）。
      postToDsh(embedFrame.value, {
        channel: "whalito",
        type: "update-progress",
        message: stageText[e.payload] ?? e.payload,
      });
    }),
  );
  unlisteners.push(
    await listen<number>("server-exited", async () => {
      server.value = await invoke<ServerStatus>("server_status");
      if (settings.value?.autoRestart && autoRestartCount.value < MAX_AUTO_RESTART) {
        autoRestartCount.value += 1;
        logs.value.push(
          `[系统] 服务器异常退出，自动重启（第 ${autoRestartCount.value}/${MAX_AUTO_RESTART} 次）`,
        );
        const r = await invoke<ServerStatus>("start_server").catch(() => null);
        if (r) server.value = r;
      } else if (autoRestartCount.value >= MAX_AUTO_RESTART) {
        error.value = `服务器连续 ${MAX_AUTO_RESTART} 次启动失败，已停止自动重试。请查看日志或重新安装 Harness。`;
      }
    }),
  );
  unlisteners.push(
    await listen<string>("tray-action", (e) => {
      if (e.payload === "start") startServer();
      else if (e.payload === "stop") stopServerTray();
      else if (e.payload === "open") {
        if (server.value.url) view.value = "embed";
        else view.value = "panel";
      }
    }),
  );
  // 桌宠请求唤起主界面：只切换视图并聚焦，不重载 iframe
  // （审批 / 提问经 SSE 实时到达，前端连接保持存活）。
  unlisteners.push(
    await listen<string | null>("pet-open-session", () => {
      if (server.value.url) {
        view.value = "embed";
      } else {
        view.value = "panel";
      }
    }),
  );
  unlisteners.push(
    await listen<string>("whalito-update", (e) => {
      postToDsh(embedFrame.value, {
        channel: "whalito",
        type: "update-progress",
        message: e.payload,
      });
    }),
  );
  window.addEventListener("message", handleWhalitoMessage);
  // 鲸仔面板右键不弹菜单；iframe 内的右键事件不会冒泡到这里，
  // 所以只影响面板自身。
  window.addEventListener("contextmenu", onPanelContextMenu);

  await Promise.all([loadSettings(), refreshLogs()]);
  platform.value = await invoke<string>("get_platform").catch(() => "windows");
  pollTimer = window.setInterval(refreshStatus, 3000);
  await runFlow();
  checkLatest();
  versionTimer = window.setInterval(checkLatest, 5 * 60 * 1000);
});

onUnmounted(() => {
  window.removeEventListener("message", handleWhalitoMessage);
  window.removeEventListener("contextmenu", onPanelContextMenu);
  if (toastTimer !== undefined) window.clearTimeout(toastTimer);
  unlisteners.forEach((u) => u());
  if (pollTimer) window.clearInterval(pollTimer);
  if (versionTimer) window.clearInterval(versionTimer);
});

// 离开内嵌页视图时收起右键菜单。
watch(view, () => {
  ctxMenu.value = null;
});

const logBox = ref<HTMLElement | null>(null);
function autoScroll() {
  requestAnimationFrame(() => {
    if (logBox.value) logBox.value.scrollTop = logBox.value.scrollHeight;
  });
}
</script>

<template>
  <!-- ============ 内嵌页面（单窗口） ============ -->
  <div v-if="view === 'embed'" class="embed">
    <iframe
      v-if="server.url"
      ref="embedFrame"
      :key="embedNonce"
      :src="server.url"
      class="embed-frame"
      @load="onEmbedLoad"
    />
    <div v-else class="embed-empty">
      <p>服务器未运行</p>
      <div class="row">
        <button class="primary" @click="startServer">启动服务器</button>
        <button @click="goPanel">返回鲸仔助手</button>
        <button @click="showSettings = true">打开设置</button>
      </div>
    </div>
  </div>

  <!-- ============ 主窗口：流程 / 面板 ============ -->
  <div v-else class="app">
    <!-- 引导式主流程 -->
    <div v-if="view === 'flow'" class="flow">
      <div class="flow-card">
        <div class="flow-brand">
          <span class="dot big" :class="phaseClass"></span>
          <div>
            <h1>鲸仔</h1>
            <p class="sub en">Whalito</p>
          </div>
        </div>

        <p v-if="stage === 'detecting'" class="flow-title">正在检测环境…</p>

        <!-- Node 缺失 / 版本过低 -->
        <div v-if="stage === 'node'">
          <p class="flow-title">
            {{ env?.found ? `当前 Node 版本过低（${env?.version}）` : "未检测到 Node.js" }}
          </p>
          <p class="hint">DeepSeek Harness 需要 Node.js ≥ 22.19.0。请选择一种安装方式：</p>
          <div class="flow-actions">
            <button
              v-if="env?.nvmFound"
              class="primary"
              :disabled="installingNode"
              @click="installNodeNvm"
            >
              {{ installingNode ? "正在安装…" : "用 nvm 安装 Node 22" }}
            </button>
            <button
              class="primary"
              :disabled="installingNode"
              @click="env?.found ? upgradeNode() : installNode()"
            >
              {{
                installingNode
                  ? "正在安装…"
                  : env?.found
                    ? platform === "windows"
                      ? "一键升级（winget）"
                      : "一键升级 Node"
                    : platform === "windows"
                      ? "一键安装（winget）"
                      : "一键安装 Node 22"
              }}
            </button>
            <button :disabled="installingNode" @click="installNodePortable">
              自定义安装目录…
            </button>
          </div>
          <p v-if="env?.nvmFound" class="hint good">已检测到 nvm（{{ env.nvmPath }}）</p>
          <p v-if="platform === 'macos'" class="hint">
            鲸仔将下载 Node.js 22 官方安装包到「~/Library/Application
            Support」，无需管理员权限。
          </p>
        </div>

        <!-- 安装 dsh -->
        <div v-if="stage === 'dsh'">
          <p class="flow-title">正在安装 DeepSeek Harness…</p>
          <div class="progress-track">
            <div class="progress-bar"></div>
          </div>
          <p class="hint">{{ stageText[installStage] ?? "准备中…" }}</p>
        </div>

        <!-- 启动服务器 -->
        <div v-if="stage === 'server'">
          <p class="flow-title">正在启动服务器…</p>
          <div class="progress-track">
            <div class="progress-bar"></div>
          </div>
          <p class="hint">首次启动可能需要一点时间，请稍候…</p>
        </div>

        <p v-if="flowError" class="banner error">{{ flowError }}</p>
        <p v-if="busy" class="banner busy">⏳ {{ busy }}</p>
        <p v-if="error && stage !== 'node'" class="banner error">{{ error }}</p>

        <div v-if="flowError" class="flow-actions">
          <button class="primary" @click="runFlow">重试</button>
          <button class="ghost" @click="goPanel">进入高级面板</button>
        </div>

        <button v-if="stage === 'node' || stage === 'detecting'" class="ghost link" @click="goPanel">
          进入高级面板
        </button>
      </div>
    </div>

    <!-- 高级/管理面板 -->
    <template v-else>
      <header class="topbar">
        <div class="brand">
          <span class="dot" :class="phaseClass"></span>
          <div>
            <h1>鲸仔</h1>
            <p class="sub en">Whalito</p>
            <p class="sub">一键安装 · 启动 · 管理你的 Harness</p>
          </div>
        </div>
        <div class="top-actions">
          <button class="ghost" @click="showSettings = true">设置</button>
        </div>
      </header>

      <div v-if="installingNode || installingDsh" class="progress-track">
        <div class="progress-bar"></div>
      </div>

      <p v-if="error" class="banner error">{{ error }}</p>
      <p v-if="notice" class="banner notice">{{ notice }}</p>
      <p v-if="busy" class="banner busy">⏳ {{ busy }}</p>

      <section class="grid">
        <div class="card">
          <h2>① 环境检测</h2>
          <ul class="kv">
            <li>
              <span>Node.js</span>
              <b :class="env?.found ? 'ok' : 'bad'">{{ env?.found ? `已安装 ${env?.version}` : "未检测到" }}</b>
            </li>
            <li v-if="env?.nodeTooOld">
              <span>版本要求</span>
              <b class="bad">需要 ≥ 22.19.0</b>
            </li>
            <li>
              <span>安装路径</span>
              <code>{{ env?.nodePath ?? "—" }}</code>
            </li>
            <li v-if="env?.nvmFound">
              <span>nvm</span>
              <code>{{ env?.nvmPath }}</code>
            </li>
          </ul>
          <div class="row">
            <button @click="refreshEnv">重新检测</button>
            <button v-if="env && !env.found" class="primary" :disabled="installingNode" @click="installNode">
              {{ installingNode ? "正在安装中…" : "一键安装 Node.js" }}
            </button>
            <button v-else-if="env?.nodeTooOld" class="primary" :disabled="installingNode" @click="upgradeNode">
              {{ installingNode ? "正在升级中…" : "升级 Node.js" }}
            </button>
            <button :disabled="installingNode" @click="installNodePortable">自定义安装目录…</button>
          </div>
        </div>

        <div class="card">
          <h2>② DeepSeek Harness</h2>
          <ul class="kv">
            <li>
              <span>安装状态</span>
              <b :class="env?.dshInstalled ? 'ok' : 'bad'">
                {{ env?.dshInstalled ? `已安装 ${env?.dshVersion ?? ""}` : "未安装" }}
              </b>
            </li>
            <li>
              <span>npm 全局前缀</span>
              <code>{{ env?.npmPrefix ?? "—" }}</code>
            </li>
            <li>
              <span>安装目录</span>
              <code>{{ env?.installPrefix ?? "—" }}</code>
            </li>
          </ul>
          <p v-if="updateAvailable" class="hint update">发现新版本 {{ latestVersion }}（当前 {{ env?.dshVersion }}）</p>
          <p v-else-if="isLatest" class="hint good">当前已是最新版本（{{ latestVersion }}）</p>
          <p class="hint">Harness 安装到程序独立目录（见“安装目录”），与系统全局 npm 隔离，更稳定。</p>
          <div class="row">
            <button v-if="env && !env.dshInstalled" class="primary" :disabled="installingDsh" @click="installDsh">
              {{ installingDsh ? "正在安装中…" : "安装 Harness" }}
            </button>
            <template v-else>
              <button v-if="updateAvailable" class="primary" :disabled="installingDsh" @click="updateDsh">
                {{ installingDsh ? "正在更新中…" : `更新到 ${latestVersion}` }}
              </button>
              <button v-else :disabled="checkingUpdate" @click="checkLatest(true)">
                {{ checkingUpdate ? "检查中…" : "检查更新" }}
              </button>
            </template>
            <button :disabled="verifying" @click="verifyDsh">{{ verifying ? "校验中…" : "校验安装" }}</button>
          </div>
        </div>

        <div class="card">
          <h2>③ 服务器</h2>
          <div class="status-line">
            <span class="dot big" :class="phaseClass"></span>
            <span class="phase">{{ serverPhaseText }}</span>
            <code v-if="server.url" class="url">{{ server.url }}</code>
          </div>
          <div class="row">
            <button v-if="server.phase === 'stopped'" class="primary" @click="startServer">启动服务器</button>
            <button v-else-if="server.phase === 'error'" class="primary" @click="startServer">重新启动</button>
            <button v-else class="danger" @click="stopServer">
              {{ server.phase === "external" && confirmingStop ? "再次点击确认停止" : "停止服务器" }}
            </button>
            <button v-if="server.phase === 'external' && confirmingStop" class="ghost" @click="confirmingStop = false">取消</button>
            <button v-if="server.phase === 'running' || server.phase === 'external'" class="ghost" @click="restartServer">重启服务器</button>
            <button v-if="server.url" class="primary" @click="openEmbedded">应用内打开</button>
            <button v-if="server.url" @click="openUrl">在浏览器打开</button>
          </div>
          <p v-if="server.phase === 'external'" class="hint warn">该服务器由外部启动；点击「停止服务器」将按端口定位并结束对应进程（需二次确认）。</p>
        </div>
      </section>

      <div v-if="showSettings && settings" class="modal-backdrop" @click.self="showSettings = false">
        <div class="modal">
          <div class="modal-head">
            <h2>设置</h2>
            <button class="ghost small" @click="showSettings = false">✕</button>
          </div>
          <div class="form">
            <label>
              <span>端口</span>
              <input v-model.number="settings.port" type="number" min="0" max="65535" />
            </label>
            <label>
              <span>npm 镜像源</span>
              <input v-model="settings.registry" type="text" placeholder="https://registry.npmjs.org" />
            </label>
            <label>
              <span>工作目录（可选）</span>
              <div class="row">
                <input v-model="settings.workspaceDir" type="text" placeholder="留空使用默认目录" />
                <button class="ghost" @click="pickWorkspaceDir">选择…</button>
              </div>
              <p class="hint">DSH 服务器的工作目录，会话里终端等相对路径以此为基准；留空使用默认目录。</p>
            </label>
            <label>
              <span>Node 安装目录</span>
              <p class="hint">{{ settings.nodeDir || "自动检测" }}（鲸仔自动检测或安装时写入，无需手动填写）</p>
            </label>
            <label>
              <span>下载目录（可选）</span>
              <div class="row">
                <input v-model="settings.downloadDir" type="text" placeholder="留空使用系统下载目录" />
                <button class="ghost" @click="pickDownloadDir">选择…</button>
              </div>
              <p class="hint">会话日志等下载的保存位置；留空使用系统下载目录。</p>
            </label>
            <label class="check">
              <input v-model="settings.autostart" type="checkbox" @change="toggleAutostart" />
              <span>开机自启本程序</span>
            </label>
            <label class="check">
              <input v-model="settings.autoRestart" type="checkbox" />
              <span>服务器异常退出后自动重启</span>
            </label>
            <label class="check">
              <input v-model="settings.petEnabled" type="checkbox" @change="togglePet" />
              <span>显示桌宠</span>
            </label>
          </div>
          <div class="row">
            <button class="primary" @click="saveSettings">保存设置</button>
            <button class="ghost" @click="showSettings = false">取消</button>
          </div>
          <p class="hint">npm 镜像源在安装/更新 Harness 时生效；国内网络慢可改为 https://registry.npmmirror.com</p>
        </div>
      </div>

      <section class="card logs">
        <div class="logs-head">
          <h2>运行日志</h2>
          <button class="ghost small" @click="clearLogs">清空</button>
        </div>
        <div ref="logBox" class="logbox">
          <div v-if="logs.length === 0" class="empty">暂无日志</div>
          <div v-for="(l, i) in logs" :key="i" class="line">{{ l }}</div>
        </div>
      </section>
    </template>
  </div>

  <!-- 内嵌 DSH 页面右键自定义菜单（复制/粘贴 ─ 刷新页面/重启服务器/显示隐藏桌宠） -->
  <div
    v-if="ctxMenu"
    class="ctx-menu"
    :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
    @contextmenu.prevent="ctxMenu = null"
  >
    <button type="button" @click="ctxCopy">复制</button>
    <button type="button" @click="ctxPaste">粘贴</button>
    <div class="ctx-sep" />
    <button type="button" @click="ctxReload">刷新页面</button>
    <button type="button" @click="ctxRestart">重启服务器</button>
    <button type="button" @click="ctxTogglePet">显示 / 隐藏桌宠</button>
  </div>

  <!-- 下载完成/复制成功提示（无路径时只显示纯文本提示，不出现文件夹按钮） -->
  <div v-if="toast" class="toast">
    <div class="toast-body">
      <span class="toast-text">{{ toast.text }}</span>
      <span v-if="toast.path" class="toast-path" :title="toast.path">{{ toast.path }}</span>
    </div>
    <div class="toast-actions">
      <button v-if="toast.path" type="button" class="toast-btn" @click="revealDownload(toast.path)">打开所在文件夹</button>
      <button type="button" class="toast-btn" @click="toast = null">✕</button>
    </div>
  </div>
</template>
