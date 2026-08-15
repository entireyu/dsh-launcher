// 握手仿真：子端（DSH 页面内的 whalito 插件组件，真实 React）+ 父端（App.vue
// 消息处理逻辑的 JS 移植），经同步消息总线互联，验证 ping → hello → 表单渲染。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const clientJs = readFileSync(join(root, "src-tauri", "whalito-dsh-settings", "client.js"), "utf8");

const req = createRequire(join(process.env.TEMP, "whalito-rt", "package.json"));
const React = req("react");
const jsxRuntime = req("react/jsx-runtime");
const { create, act } = req("react-test-renderer");

// ── 父端（App.vue 逻辑移植） ──
const parentSettings = {
  port: 3080, registry: "https://registry.npmjs.org", autostart: false,
  autoRestart: true, workspaceDir: null, nodeDir: null, petEnabled: true,
};
const parentStatus = { phase: "running", url: "http://127.0.0.1:3080", pid: 1234 };
const parentUrl = parentStatus.url;
const parentPings = [];
const parentLog = [];

// 方向约定：子端 window.parent.postMessage → 送达父端处理器；
// 父端 frame.contentWindow.postMessage → 送达子端监听器。
const parentWindowStub = { postMessage(msg) { deliverToParent(msg); } };
const childWindowProxy = { postMessage(msg) { deliverToChild(msg); } };
const frameStub = { contentWindow: childWindowProxy };

function deliverToChild(msg) {
  // 子端 onMessage：source = 父窗口桩
  childListeners.forEach((fn) => fn({ data: msg, source: parentWindowStub }));
}
function deliverToParent(msg) {
  parentHandler({ data: msg, origin: new URL(parentUrl).origin, source: childWindowProxy });
}
function pushSnapshot() {
  frameStub.contentWindow.postMessage({
    channel: "whalito", type: "hello",
    settings: parentSettings, status: parentStatus,
  });
}
function parentHandler(event) {
  const origin = new URL(parentUrl).origin;
  if (event.origin !== origin) return;
  const d = event.data;
  if (typeof d !== "object" || d === null || d.channel !== "whalito" || typeof d.type !== "string") return;
  if (d.type === "ping") {
    parentPings.push(d);
    parentLog.push("收到 ping");
    pushSnapshot();
  }
}

// ── 子端：真实 bundle + 真实 React ──
const childListeners = [];
const captured = { handoff: null };
const childWindow = {
  __ModuleLoader__: { load(h) { captured.handoff = h; } },
  parent: parentWindowStub,
  addEventListener(type, fn) { if (type === "message") childListeners.push(fn); },
  removeEventListener() {},
  setInterval: () => 0,   // 测试环境忽略重试定时器
  clearInterval: () => {},
  console,
};
new Function("window", clientJs)(childWindow);

const requireStub = (spec) => {
  if (spec === "react") return React;
  if (spec === "react/jsx-runtime") return jsxRuntime;
  throw new Error("unexpected external: " + spec);
};
const mod = captured.handoff.factory(requireStub);
const registrations = [];
mod.apply({
  slots: {
    inject(name, factory) { registrations.push(factory()); },
    register(options, component) { return { ...options, component }; },
  },
});
const Component = registrations[0].component;

let renderer;
act(() => {
  renderer = create(jsxRuntime.jsx(Component, {}));
});
// 挂载 effect 已同步发送 ping → 父端同步回 hello → 需要一次 act 冲刷 setState
act(() => {});

const text = () => JSON.stringify(renderer.toJSON());
assert.ok(parentPings.length >= 1, "子端挂载应发出 ping");
assert.ok(parentLog.includes("收到 ping"), "父端应收到 ping");
assert.ok(!text().includes("正在连接鲸仔"), "握手后不应停留在连接中");
assert.ok(text().includes("端口"), "握手后应渲染完整表单");
assert.ok(text().includes("运行中"), "应显示服务器状态");
console.log("ok: 握手完成（ping → hello → 表单渲染），父端收到", parentPings.length, "次 ping");

// 父端推送 settings 更新 → 子端 draft 同步（未编辑时）
act(() => {
  const updated = { ...parentSettings, port: 9090 };
  frameStub.contentWindow.postMessage({ channel: "whalito", type: "settings", settings: updated, status: parentStatus });
});
assert.ok(text().includes("9090"), "settings 更新应同步到表单");
console.log("ok: settings 更新同步");

// 回归防护：Vue reactive Proxy 过 structuredClone 会抛 DataCloneError（真实根因），
// toPlain 的 JSON 深拷贝形式必须可克隆。
{
  const { ref: vueRef } = req("vue");
  const proxy = vueRef({ port: 30080, s: null, b: false }).value;
  assert.throws(() => structuredClone(proxy), "reactive proxy 应无法克隆（复现根因）");
  const plain = JSON.parse(JSON.stringify(proxy));
  assert.deepEqual(structuredClone(plain), plain, "toPlain 形式应可克隆");
  console.log("ok: 代理克隆陷阱回归防护");
}

console.log("handshake simulation: all passed");
