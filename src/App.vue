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
  prefixFallback: boolean;
  dshInstalled: boolean;
  dshVersion: string | null;
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
  autoStartServer: boolean;
  autoRestart: boolean;
  workspaceDir: string | null;
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

const unlisteners: UnlistenFn[] = [];
let pollTimer: number | undefined;

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
  env.value = await invoke<EnvInfo>("detect_env");
}

async function refreshStatus() {
  try {
    server.value = await invoke<ServerStatus>("server_status");
    if (server.value.phase !== "external") confirmingStop.value = false;
  } catch {
    /* 忽略瞬时错误 */
  }
}

async function refreshAll() {
  await Promise.all([refreshEnv(), refreshStatus()]);
}

async function installNode() {
  const r = await wrap("正在安装 Node.js…", () => invoke<EnvInfo>("install_node"));
  if (r) env.value = r;
}

async function installDsh() {
  const r = await wrap("正在安装 DeepSeek Harness…", () =>
    invoke<EnvInfo>("install_dsh"),
  );
  if (r) env.value = r;
}

async function updateDsh() {
  const r = await wrap("正在更新 DeepSeek Harness…", () =>
    invoke<EnvInfo>("update_dsh"),
  );
  if (r) env.value = r;
}

async function verifyDsh() {
  const r = await wrap("正在校验…", () => invoke<string>("verify_dsh"));
  if (r) notice.value = r;
}

async function startServer() {
  const r = await wrap("正在启动…", () => invoke<ServerStatus>("start_server"));
  if (r) server.value = r;
}

async function stopServer() {
  if (server.value.phase === "external" && !confirmingStop.value) {
    confirmingStop.value = true;
    return;
  }
  confirmingStop.value = false;
  const r = await wrap("正在停止…", () => invoke<ServerStatus>("stop_server"));
  if (r) server.value = r;
}

async function openUrl() {
  if (server.value.url) {
    await invoke("open_url", { url: server.value.url });
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
}

async function saveSettings() {
  if (!settings.value) return;
  const r = await wrap("正在保存设置…", () =>
    invoke<Settings>("save_settings", { value: settings.value }),
  );
  if (r) {
    settings.value = r;
    notice.value = "设置已保存";
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
    await listen<number>("server-exited", async () => {
      server.value = await invoke<ServerStatus>("server_status");
      if (settings.value?.autoRestart) {
        const r = await invoke<ServerStatus>("start_server").catch(() => null);
        if (r) server.value = r;
      }
    }),
  );
  unlisteners.push(
    await listen<string>("tray-action", (e) => {
      if (e.payload === "start") startServer();
      else if (e.payload === "stop") stopServer();
      else if (e.payload === "open") openUrl();
    }),
  );

  await Promise.all([refreshAll(), loadSettings(), refreshLogs()]);
  pollTimer = window.setInterval(refreshStatus, 3000);
});

onUnmounted(() => {
  unlisteners.forEach((u) => u());
  if (pollTimer) window.clearInterval(pollTimer);
});

const logBox = ref<HTMLElement | null>(null);
function autoScroll() {
  requestAnimationFrame(() => {
    if (logBox.value) logBox.value.scrollTop = logBox.value.scrollHeight;
  });
}
</script>

<template>
  <div class="app">
    <header class="topbar">
      <div class="brand">
        <span class="dot" :class="phaseClass"></span>
        <div>
          <h1>DeepSeek Harness 助手</h1>
          <p class="sub">一键安装 · 启动 · 管理你的 Harness</p>
        </div>
      </div>
      <div class="top-actions">
        <button class="ghost" @click="showSettings = !showSettings">设置</button>
      </div>
    </header>

    <p v-if="error" class="banner error">{{ error }}</p>
    <p v-if="notice" class="banner notice">{{ notice }}</p>
    <p v-if="busy" class="banner busy">⏳ {{ busy }}</p>

    <section class="grid">
      <div class="card">
        <h2>① 环境检测</h2>
        <ul class="kv">
          <li>
            <span>Node.js</span>
            <b :class="env?.found ? 'ok' : 'bad'">{{ env?.found ? `已安装 v${env?.version}` : "未检测到" }}</b>
          </li>
          <li>
            <span>安装路径</span>
            <code>{{ env?.nodePath ?? "—" }}</code>
          </li>
        </ul>
        <div class="row">
          <button @click="refreshEnv">重新检测</button>
          <button v-if="env && !env.found" class="primary" @click="installNode">一键安装 Node.js</button>
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
        <p v-if="env?.prefixFallback" class="hint warn">全局前缀不可写，已改用用户目录安装（无需管理员权限）。</p>
        <div class="row">
          <button v-if="env && !env.dshInstalled" class="primary" @click="installDsh">安装 Harness</button>
          <button v-else @click="updateDsh">更新到最新</button>
          <button @click="verifyDsh">校验安装</button>
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
          <button v-if="server.url" @click="openUrl">在浏览器打开</button>
        </div>
        <p v-if="server.phase === 'external'" class="hint warn">该服务器由外部启动；点击「停止服务器」将按端口定位并结束对应进程（需二次确认）。</p>
      </div>
    </section>

    <section v-if="showSettings && settings" class="card settings">
      <h2>设置</h2>
      <div class="form">
        <label>
          <span>端口</span>
          <input v-model.number="settings!.port" type="number" min="0" max="65535" />
        </label>
        <label>
          <span>npm 镜像源</span>
          <input v-model="settings!.registry" type="text" placeholder="https://registry.npmjs.org" />
        </label>
        <label>
          <span>工作目录（可选）</span>
          <input v-model="settings!.workspaceDir" type="text" placeholder="留空使用默认目录" />
        </label>
        <label class="check">
          <input v-model="settings!.autostart" type="checkbox" @change="toggleAutostart" />
          <span>开机自启本程序</span>
        </label>
        <label class="check">
          <input v-model="settings!.autoStartServer" type="checkbox" />
          <span>启动程序时自动启动服务器</span>
        </label>
        <label class="check">
          <input v-model="settings!.autoRestart" type="checkbox" />
          <span>服务器异常退出后自动重启</span>
        </label>
      </div>
      <div class="row">
        <button class="primary" @click="saveSettings">保存设置</button>
      </div>
      <p class="hint">npm 镜像源在安装/更新 Harness 时生效；国内网络慢可改为 https://registry.npmmirror.com</p>
    </section>

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
  </div>
</template>
