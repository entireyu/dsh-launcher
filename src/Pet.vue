<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { PhysicalPosition } from "@tauri-apps/api/dpi";

interface TodoItem {
  content: string;
  status: "pending" | "in_progress" | "completed";
}

interface Goal {
  id?: string;
  revision?: number;
  objective?: string;
  phase?: "active" | "paused" | "blocked" | "complete";
}

interface PetSession {
  sessionId: string;
  running: boolean;
  blank: boolean;
  title: string | null;
  goal: Goal | null;
  todos: TodoItem[] | null;
  isSubagent: boolean;
}

interface PetState {
  phase: string;
  sessions: PetSession[];
  runningCount: number;
  subagentCount: number;
  error: string | null;
  /** 待查看通知（任务完成 / 被阻塞 / 被中断）；打开主界面后由后端清除。 */
  notice: PetNotice | null;
}

/** 桌宠待查看通知：设置后持续展示，直到用户打开主界面才清除。 */
interface PetNotice {
  kind: "completed" | "blocked" | "interrupted";
  title: string | null;
  goal: string | null;
}

interface PetAlert {
  kind: "approval" | "question";
  key: string;
  rpcId: string;
  sessionId: string;
  approvalId?: string;
  toolName?: string;
  reason?: string;
  questions?: unknown;
}

/** 桌宠样式契约（~/.dsh/pet-style.json，可经 DSH 或命令调整）。 */
interface PetStyle {
  schemaVersion: number;
  size: number;
  position: { x: number; y: number } | null;
  avatar: string | null;
  accent: string;
  bubble: { bg: string; fg: string; sub: string; fontSize: number };
  animations: { bob: boolean; float: boolean; attention: boolean };
}

const state = ref<PetState | null>(null);
const alerts = ref<PetAlert[]>([]);
const menuOpen = ref(false);
const menuPos = ref({ x: 0, y: 0 });
const style = ref<PetStyle | null>(null);
const dragState = ref<{
  x: number;
  y: number;
  winX: number;
  winY: number;
  scale: number;
} | null>(null);
const dragMoved = ref(false);

const runningSessions = computed(() =>
  (state.value?.sessions ?? []).filter((s) => s.running && !s.blank && !s.isSubagent),
);

const primary = computed(() => {
  const withGoal = runningSessions.value.find((s) => s.goal?.objective);
  return withGoal ?? runningSessions.value[0] ?? null;
});

const goalObjective = computed(() => primary.value?.goal?.objective ?? null);
const goalPhase = computed(() => primary.value?.goal?.phase ?? null);

const todos = computed(() => primary.value?.todos ?? null);
const todoTotal = computed(() => todos.value?.length ?? 0);
const todoDone = computed(
  () => todos.value?.filter((t) => t.status === "completed").length ?? 0,
);
const todoInProgress = computed(
  () => todos.value?.find((t) => t.status === "in_progress")?.content ?? null,
);

const approvals = computed(() => alerts.value.filter((a) => a.kind === "approval"));
const questions = computed(() => alerts.value.filter((a) => a.kind === "question"));
const firstApproval = computed(() => approvals.value[0] ?? null);
const alertCount = computed(() => alerts.value.length);

const phaseLabel = computed(() => {
  if (!state.value) return "正在连接…";
  if (state.value.phase === "error") return state.value.error ?? "无法连接服务器";
  if (state.value.phase === "stopped") return "服务器未运行";
  return null;
});

/** 服务器正常且无任何任务/后台代理在跑（"空闲中"的判定条件）。 */
const idle = computed(
  () => !!state.value && state.value.phase === "running" && state.value.runningCount === 0,
);

/** 任务被中断通知：服务器停止或恢复空闲时展示，直到打开主界面才清除；
 *  有任务在跑时优先展示任务进度，连接异常时优先展示错误信息。 */
const interruptedNotice = computed(() => {
  const st = state.value;
  const n = st?.notice;
  if (!st || n?.kind !== "interrupted") return null;
  if (st.runningCount > 0 || st.phase === "error") return null;
  return n;
});

/** 任务完成/被阻塞通知：仅在空闲时展示（有任务在跑时展示任务进度）。 */
const completedNotice = computed(() => {
  const n = state.value?.notice;
  if (!n || n.kind === "interrupted" || !idle.value) return null;
  return n;
});

/** 待查看通知触发注意力动画（与审批/提问同款提醒）。 */
const attention = computed(
  () =>
    (alertCount.value > 0 || interruptedNotice.value !== null || completedNotice.value !== null) &&
    anim.value.attention,
);

const bubbleTitle = computed(() => {
  const a = firstApproval.value;
  if (a) return a.toolName ? `需要确认：${a.toolName}` : "需要你确认";
  if (questions.value.length > 0) return "需要你回答";
  if (interruptedNotice.value) return "任务已中断";
  if (phaseLabel.value) return phaseLabel.value;
  const done = completedNotice.value;
  if (done) return done.kind === "blocked" ? "任务被阻塞" : "任务已完成 🎉";
  if (goalObjective.value) return goalObjective.value;
  if (primary.value?.title) return primary.value.title;
  if (idle.value) return "空闲中";
  return "工作中…";
});

const bubbleSub = computed(() => {
  const a = firstApproval.value;
  if (a) return a.reason ?? "点击打开应用确认";
  if (questions.value.length > 0) return "点击打开应用回答";
  if (interruptedNotice.value) return "服务器已断开，点击打开面板查看";
  if (phaseLabel.value) return null;
  const done = completedNotice.value;
  if (done) {
    const what = done.goal ?? done.title;
    if (done.kind === "blocked") {
      return what ? `被阻塞：${what}` : "点击打开面板处理";
    }
    return what ? `已完成：${what}` : "点击打开面板查看详情";
  }
  if (idle.value) return null;
  const parts: string[] = [];
  if (state.value) {
    parts.push(`${state.value.runningCount} 个任务运行中`);
    if (state.value.subagentCount > 0) parts.push(`${state.value.subagentCount} 个后台代理`);
  }
  if (todoTotal.value > 0) parts.push(`待办 ${todoDone.value}/${todoTotal.value}`);
  if (todoInProgress.value) parts.push(`进行中：${todoInProgress.value}`);
  if (goalPhase.value) parts.push(goalPhase.value);
  return parts.join(" · ");
});

// 样式契约驱动的外观（全部可被 pet-style.json 覆盖）。
const whaleStyle = computed(() => {
  const size = style.value?.size ?? 96;
  return { width: `${size}px`, height: `${size}px` };
});
const avatarSrc = computed(() => style.value?.avatar || "/logo.png");
const accentStyle = computed(() => ({ background: style.value?.accent ?? "#f87171" }));
const bubbleStyle = computed(() =>
  style.value
    ? {
        background: style.value.bubble.bg,
        borderColor: style.value.bubble.bg,
        color: style.value.bubble.fg,
        fontSize: `${style.value.bubble.fontSize}px`,
      }
    : {},
);
const subStyle = computed(() => ({ color: style.value?.bubble.sub ?? "#9aa3b2" }));
const anim = computed(() => style.value?.animations ?? { bob: true, float: true, attention: true });

function onAvatarError() {
  // 头像加载失败 → 本地回退内置 logo（不写回契约文件）。
  if (style.value?.avatar) {
    style.value = { ...style.value, avatar: null };
  }
}

// 拖拽：手动移动窗口（透明窗口下 startDragging/data-tauri-drag-region
// 在 Windows 均不可靠）。以按下点为锚、按缩放系数换算物理坐标，
// 4px 阈值内算点击；onMoved 事件照常触发位置持久化。
async function onWhaleMouseDown(e: MouseEvent) {
  if (e.button !== 0) return;
  const win = getCurrentWindow();
  const [pos, scale] = await Promise.all([win.outerPosition(), win.scaleFactor()]);
  dragState.value = {
    x: e.screenX,
    y: e.screenY,
    winX: pos.x,
    winY: pos.y,
    scale,
  };
  dragMoved.value = false;
  window.removeEventListener("mousemove", onWhaleMouseMove);
  window.removeEventListener("mouseup", onWhaleMouseUp);
  window.addEventListener("mousemove", onWhaleMouseMove);
  window.addEventListener("mouseup", onWhaleMouseUp);
}

// 拖拽节流：mousemove 只更新"期望位置"，每帧最多执行一次 setPosition
// （requestAnimationFrame 合并），避免逐事件 await IPC 造成的队列积压滞后。
let pendingTarget: { x: number; y: number } | null = null;
let dragRafId: number | null = null;

function applyPendingPosition() {
  dragRafId = null;
  const t = pendingTarget;
  if (!t) return;
  pendingTarget = null;
  getCurrentWindow()
    .setPosition(new PhysicalPosition(t.x, t.y))
    .catch(() => {});
}

async function onWhaleMouseMove(e: MouseEvent) {
  const d = dragState.value;
  if (!d) return;
  const dx = (e.screenX - d.x) * d.scale;
  const dy = (e.screenY - d.y) * d.scale;
  if (!dragMoved.value && Math.abs(dx) <= 4 * d.scale && Math.abs(dy) <= 4 * d.scale) {
    return;
  }
  dragMoved.value = true;
  pendingTarget = { x: d.winX + dx, y: d.winY + dy };
  if (dragRafId === null) {
    dragRafId = requestAnimationFrame(applyPendingPosition);
  }
}

function onWhaleMouseUp() {
  dragState.value = null;
  window.removeEventListener("mousemove", onWhaleMouseMove);
  window.removeEventListener("mouseup", onWhaleMouseUp);
  // 松手时把最后一次目标位置落盘（取消未执行的帧，直接应用）。
  if (dragRafId !== null) {
    cancelAnimationFrame(dragRafId);
    dragRafId = null;
  }
  applyPendingPosition();
}

function onWhaleClick() {
  if (dragMoved.value) {
    dragMoved.value = false;
    return;
  }
  open(primary.value?.sessionId ?? null);
}

const unlisteners: UnlistenFn[] = [];

function upsertAlert(a: PetAlert) {
  const i = alerts.value.findIndex((x) => x.key === a.key);
  if (i >= 0) alerts.value[i] = a;
  else alerts.value.push(a);
}

function clearAlert(key: string) {
  alerts.value = alerts.value.filter((x) => x.key !== key);
}

async function open(sessionId: string | null) {
  menuOpen.value = false;
  await invoke("pet_open_session", { sessionId });
}

async function respond(a: PetAlert | null, outcome: "allowed-once" | "rejected") {
  if (!a?.approvalId) return;
  try {
    await invoke("pet_respond", {
      rpcId: a.rpcId,
      sessionId: a.sessionId,
      approvalId: a.approvalId,
      outcome,
    });
  } catch {
    /* 忽略（可能是已过期 / 服务器未运行） */
  }
}

async function hidePet() {
  menuOpen.value = false;
  await invoke("pet_set_enabled", { enabled: false });
}

// 右键菜单：与托盘菜单同款动作。启动/停止/在浏览器打开沿用托盘事件通道，
// 由主窗口统一处理；打开面板/退出走新增命令。
function onContextMenu(e: MouseEvent) {
  const W = 200;
  const H = 250;
  const MENU_W = 150;
  const MENU_H = 210;
  menuPos.value = {
    x: Math.min(Math.max(0, e.clientX), W - MENU_W),
    y: Math.min(Math.max(0, e.clientY), H - MENU_H),
  };
  menuOpen.value = true;
}

function trayAction(action: string) {
  menuOpen.value = false;
  emit("tray-action", action).catch(() => {});
}

async function showMainWindow() {
  menuOpen.value = false;
  await invoke("show_main_window");
}

async function quitApp() {
  menuOpen.value = false;
  await invoke("quit_app");
}

onMounted(async () => {
  invoke("bridge_diag", { line: "pet 页 onMounted 开始" }).catch(() => {});
  let firstStateLogged = false;
  unlisteners.push(
    await listen<PetState>("pet-state", (e) => {
      state.value = e.payload;
      if (!firstStateLogged) {
        firstStateLogged = true;
        invoke("bridge_diag", { line: `pet 页收到状态事件：${e.payload.phase}` }).catch(() => {});
      }
    }),
    await listen<PetAlert>("pet-alert", (e) => upsertAlert(e.payload)),
    await listen<string>("pet-alert-clear", (e) => clearAlert(e.payload)),
    await listen<PetStyle>("pet-style", (e) => {
      style.value = e.payload;
    }),
  );
  invoke("bridge_diag", { line: "pet 页监听器注册完成" }).catch(() => {});
  try {
    state.value = await invoke<PetState | null>("pet_status");
    style.value = await invoke<PetStyle>("pet_get_style");
    invoke("bridge_diag", {
      line: `pet_status 返回：${state.value ? state.value.phase : "null"}`,
    }).catch(() => {});
  } catch (e) {
    invoke("bridge_diag", { line: `pet_status invoke 失败：${String(e)}` }).catch(() => {});
  }
  // 兜底轮询：事件丢失 / 快照为空时，每 3s 主动拉取一次。
  const fallbackTimer = window.setInterval(async () => {
    if (!state.value) {
      try {
        state.value = await invoke<PetState | null>("pet_status");
      } catch {
        /* 忽略 */
      }
    }
  }, 3000);
  unlisteners.push(() => window.clearInterval(fallbackTimer));
  invoke("bridge_diag", { line: "pet 页 onMounted 完成" }).catch(() => {});
  // 拖拽结束位置持久化（防抖 500ms 写回 pet-style.json）。
  let moveTimer: number | undefined;
  unlisteners.push(
    await getCurrentWindow().onMoved(({ payload }) => {
      if (moveTimer) window.clearTimeout(moveTimer);
      moveTimer = window.setTimeout(() => {
        invoke("pet_set_position", { x: payload.x, y: payload.y }).catch(() => {});
      }, 500);
    }),
  );
});

onUnmounted(() => {
  unlisteners.forEach((u) => u());
});

// 页面级错误兜底上报（诊断用）。
window.addEventListener("error", (e) => {
  invoke("bridge_diag", { line: `pet 页 JS 错误：${e.message}` }).catch(() => {});
});
window.addEventListener("unhandledrejection", (e) => {
  invoke("bridge_diag", { line: `pet 页未处理 Promise：${String(e.reason)}` }).catch(() => {});
});
</script>

<template>
  <div class="pet" @contextmenu.prevent="onContextMenu" @click="menuOpen = false">
    <!-- 提示气泡 -->
    <div
      v-if="bubbleTitle"
      class="bubble"
      :class="{ float: anim.float }"
      :style="bubbleStyle"
      @click="open(primary?.sessionId ?? null)"
    >
      <div class="bubble-title">{{ bubbleTitle }}</div>
      <div v-if="bubbleSub" class="bubble-sub" :style="subStyle">{{ bubbleSub }}</div>
      <div v-if="approvals.length > 0" class="bubble-actions" @click.stop>
        <button class="allow" @click="respond(firstApproval, 'allowed-once')">允许</button>
        <button class="reject" @click="respond(firstApproval, 'rejected')">拒绝</button>
      </div>
    </div>

    <!-- 鲸仔本体（按住拖拽 / 轻点打开主界面） -->
    <div
      class="whale"
      :class="{ bob: anim.bob, attention }"
      :style="whaleStyle"
      @mousedown="onWhaleMouseDown"
      @click="onWhaleClick"
    >
      <img :src="avatarSrc" alt="鲸仔" draggable="false" @error="onAvatarError" />
      <span v-if="alertCount > 0" class="badge" :style="accentStyle">{{ alertCount }}</span>
    </div>

    <!-- 右键菜单（与托盘同款：打开面板/启动/停止/浏览器打开/隐藏桌宠/退出） -->
    <div
      v-if="menuOpen"
      class="menu"
      :style="{ left: menuPos.x + 'px', top: menuPos.y + 'px' }"
      @click.stop
    >
      <button @click="showMainWindow">打开面板</button>
      <button @click="trayAction('start')">启动服务器</button>
      <button @click="trayAction('stop')">停止服务器</button>
      <button @click="trayAction('open')">在浏览器打开</button>
      <button @click="hidePet">隐藏桌宠</button>
      <button @click="quitApp">退出</button>
    </div>
  </div>
</template>

<style scoped>
.pet {
  position: relative;
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-end;
  /* 底部预留动画余量：鲸仔贴窗口底边时，attention 的 scale 会越界被裁切。 */
  padding-bottom: 14px;
  font-family: "Segoe UI", "Microsoft YaHei", system-ui, sans-serif;
}

.bubble {
  position: absolute;
  top: 8px;
  left: 50%;
  transform: translateX(-50%);
  width: 188px;
  /* 气泡（对话框）始终绘制在鲸仔之上：即使内容较长与鲸仔重叠，
     也不被鲸仔的深色圆形底遮住。 */
  z-index: 2;
  background: #171a21;
  border: 1px solid #2a2f3a;
  border-radius: 12px;
  padding: 8px 10px;
  color: #e8eaf0;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
  cursor: pointer;
  text-align: left;
}

.bubble.float {
  animation: float 3s ease-in-out infinite;
}

.bubble::after {
  content: "";
  position: absolute;
  bottom: -8px;
  left: 50%;
  transform: translateX(-50%);
  border-left: 8px solid transparent;
  border-right: 8px solid transparent;
  border-top: 8px solid #171a21;
}

.bubble-title {
  font-size: inherit;
  font-weight: 600;
  line-height: 1.35;
  word-break: break-all;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.bubble-sub {
  margin-top: 3px;
  font-size: 11px;
  color: #9aa3b2;
  word-break: break-all;
  /* 副文本限 3 行：防止超长审批理由把气泡撑出窗口被裁切。 */
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.bubble-actions {
  display: flex;
  gap: 6px;
  margin-top: 6px;
}

.bubble-actions button {
  flex: 1;
  border: 1px solid #2a2f3a;
  border-radius: 6px;
  padding: 4px 0;
  font-size: 12px;
  cursor: pointer;
  background: #1e222b;
  color: #e8eaf0;
}

.bubble-actions button.allow {
  background: #34d399;
  border-color: #34d399;
  color: #06281c;
}

.bubble-actions button.reject {
  background: transparent;
  border-color: #f87171;
  color: #f87171;
}

.whale {
  position: relative;
  z-index: 1;
  cursor: pointer;
}

.whale.bob {
  animation: bob 2.6s ease-in-out infinite;
}

.whale img {
  width: 100%;
  height: 100%;
  object-fit: contain;
  border-radius: 50%;
  background: rgba(15, 17, 21, 0.35);
  pointer-events: none;
}

.whale.attention {
  animation: bounce 0.6s ease-in-out infinite;
}

.badge {
  position: absolute;
  top: -2px;
  right: -2px;
  min-width: 20px;
  height: 20px;
  padding: 0 5px;
  border-radius: 10px;
  background: #f87171;
  color: #fff;
  font-size: 12px;
  font-weight: 700;
  line-height: 20px;
  text-align: center;
  box-shadow: 0 2px 8px rgba(248, 113, 113, 0.6);
}

.menu {
  position: absolute;
  z-index: 10;
  display: flex;
  flex-direction: column;
  gap: 2px;
  background: #171a21;
  border: 1px solid #2a2f3a;
  border-radius: 10px;
  padding: 4px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
}

.menu button {
  text-align: left;
  white-space: nowrap;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: #e8eaf0;
  padding: 5px 10px;
  font-size: 12px;
  cursor: pointer;
}

.menu button:hover {
  background: #1e222b;
}

@keyframes bob {
  0%,
  100% {
    transform: translateY(0);
  }
  50% {
    transform: translateY(-6px);
  }
}

@keyframes bounce {
  0%,
  100% {
    transform: translateY(0) scale(1);
  }
  50% {
    transform: translateY(-10px) scale(1.08);
  }
}

@keyframes float {
  0%,
  100% {
    transform: translateX(-50%) translateY(0);
  }
  50% {
    transform: translateX(-50%) translateY(-4px);
  }
}
</style>
