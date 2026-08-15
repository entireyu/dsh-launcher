<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

const state = ref<PetState | null>(null);
const alerts = ref<PetAlert[]>([]);
const menuOpen = ref(false);

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
  if (!state.value || state.value.phase === "stopped") return "服务器未运行";
  if (state.value.runningCount === 0) return "空闲中";
  return null;
});

const bubbleTitle = computed(() => {
  const a = firstApproval.value;
  if (a) return a.toolName ? `需要确认：${a.toolName}` : "需要你确认";
  if (questions.value.length > 0) return "需要你回答";
  if (phaseLabel.value) return phaseLabel.value;
  if (goalObjective.value) return goalObjective.value;
  if (primary.value?.title) return primary.value.title;
  return "工作中…";
});

const bubbleSub = computed(() => {
  if (firstApproval.value) return firstApproval.value.reason ?? "点击打开应用确认";
  if (questions.value.length > 0) return "点击打开应用回答";
  if (phaseLabel.value) return null;
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

onMounted(async () => {
  unlisteners.push(
    await listen<PetState>("pet-state", (e) => {
      state.value = e.payload;
    }),
    await listen<PetAlert>("pet-alert", (e) => upsertAlert(e.payload)),
    await listen<string>("pet-alert-clear", (e) => clearAlert(e.payload)),
  );
  try {
    state.value = await invoke<PetState | null>("pet_status");
  } catch {
    /* 忽略 */
  }
});

onUnmounted(() => {
  unlisteners.forEach((u) => u());
});
</script>

<template>
  <div class="pet" @contextmenu.prevent="menuOpen = true">
    <!-- 提示气泡 -->
    <div v-if="bubbleTitle" class="bubble" @click="open(primary?.sessionId ?? null)">
      <div class="bubble-title">{{ bubbleTitle }}</div>
      <div v-if="bubbleSub" class="bubble-sub">{{ bubbleSub }}</div>
      <div v-if="approvals.length > 0" class="bubble-actions" @click.stop>
        <button class="allow" @click="respond(firstApproval, 'allowed-once')">允许</button>
        <button class="reject" @click="respond(firstApproval, 'rejected')">拒绝</button>
      </div>
    </div>

    <!-- 鲸仔本体（可拖拽） -->
    <div
      class="whale"
      :class="{ attention: alertCount > 0 }"
      data-tauri-drag-region
      @click="open(primary?.sessionId ?? null)"
    >
      <img src="/logo.png" alt="鲸仔" draggable="false" />
      <span v-if="alertCount > 0" class="badge">{{ alertCount }}</span>
    </div>

    <!-- 右键菜单 -->
    <div v-if="menuOpen" class="menu" @click.stop>
      <button @click="open(null)">打开面板</button>
      <button @click="hidePet">隐藏桌宠</button>
    </div>
  </div>
</template>

<style scoped>
.pet {
  position: relative;
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-end;
  padding-bottom: 14px;
  font-family: "Segoe UI", "Microsoft YaHei", system-ui, sans-serif;
}

.bubble {
  position: absolute;
  top: 8px;
  left: 50%;
  transform: translateX(-50%);
  width: 188px;
  background: #171a21;
  border: 1px solid #2a2f3a;
  border-radius: 12px;
  padding: 8px 10px;
  color: #e8eaf0;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
  cursor: pointer;
  text-align: left;
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
  font-size: 12px;
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
  width: 96px;
  height: 96px;
  cursor: pointer;
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
  bottom: 116px;
  right: 8px;
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
