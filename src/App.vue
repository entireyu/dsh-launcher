<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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
  autostart: boolean;
  autoRestart: boolean;
  workspaceDir: string | null;
  nodeDir: string | null;
  petEnabled: boolean;
}

const env = ref<EnvInfo | null>(null);
const server = ref<ServerStatus>({ phase: "stopped", url: null, pid: null });
const settings = ref<Settings | null>(null);
const logs = ref<string[]>([]);
const busy = ref<string | null>(null);
const error = ref<string>("");
const notice = ref<string>("");
const showSettings = ref(false);
const confirmingStop = ref(false);
const autoRestartCount = ref(0);
const MAX_AUTO_RESTART = 3;

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

const fabOpen = ref(false);
const embedNonce = ref(0);

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
  fabOpen.value = false;
  view.value = "embed";
}

// 悬浮按钮动作
async function fabStart() {
  fabOpen.value = false;
  await startServer();
  embedNonce.value += 1;
}
async function fabStop() {
  fabOpen.value = false;
  await doStop();
  view.value = "panel";
}
async function fabRestart() {
  fabOpen.value = false;
  await restartServer();
  embedNonce.value += 1;
}
function fabSettings() {
  fabOpen.value = false;
  view.value = "panel";
  showSettings.value = true;
}

async function refreshLogs() {
  logs.value = await invoke<string[]>("get_logs");
}

function clearLogs() {
  logs.value = [];
}

async function loadSettings() {
  settings.value = await invoke<Settings>("get_settings");
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
  // 桌宠请求唤起对应会话：切到内嵌视图并重载 iframe，
  // 让待处理的审批 / 提问随前端重连自动浮出。
  unlisteners.push(
    await listen<string | null>("pet-open-session", () => {
      if (server.value.url) {
        view.value = "embed";
        embedNonce.value += 1;
      } else {
        view.value = "panel";
      }
    }),
  );

  await Promise.all([loadSettings(), refreshLogs()]);
  pollTimer = window.setInterval(refreshStatus, 3000);
  await runFlow();
  checkLatest();
  versionTimer = window.setInterval(checkLatest, 5 * 60 * 1000);
});

onUnmounted(() => {
  unlisteners.forEach((u) => u());
  if (pollTimer) window.clearInterval(pollTimer);
  if (versionTimer) window.clearInterval(versionTimer);
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
    <iframe v-if="server.url" :key="embedNonce" :src="server.url" class="embed-frame" />
    <div v-else class="embed-empty">
      <p>服务器未运行</p>
      <button class="primary" @click="fabStart">启动服务器</button>
    </div>
    <div class="fab">
      <button class="fab-btn" @click="fabOpen = !fabOpen">⚙</button>
      <div v-if="fabOpen" class="fab-menu">
        <button @click="goPanel">返回助手</button>
        <button @click="fabStart">启动服务器</button>
        <button @click="fabStop">停止服务器</button>
        <button @click="fabRestart">重启服务器</button>
        <button @click="fabSettings">打开设置</button>
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
              {{ installingNode ? "正在安装…" : env?.found ? "一键升级（winget）" : "一键安装（winget）" }}
            </button>
            <button :disabled="installingNode" @click="installNodePortable">
              自定义安装目录…
            </button>
          </div>
          <p v-if="env?.nvmFound" class="hint good">已检测到 nvm（{{ env.nvmPath }}）</p>
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
              <input v-model="settings.workspaceDir" type="text" placeholder="留空使用默认目录" />
            </label>
            <label>
              <span>Node 安装目录（可选）</span>
              <input v-model="settings.nodeDir" type="text" placeholder="留空自动检测" />
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
</template>
