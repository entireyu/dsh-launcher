// 交互调试：用 react-test-renderer 驱动"鲸仔"设置分区组件，
// 覆盖 握手→表单渲染→编辑→保存/校验 的完整路径。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const clientJs = readFileSync(join(root, "src-tauri", "whalito-dsh-settings", "client.js"), "utf8");

// 临时安装的 react 18.3.1 + react-test-renderer（与 dsh 同版本，实例一致）。
const req = createRequire(join(process.env.TEMP, "whalito-rt", "package.json"));
const React = req("react");
const jsxRuntime = req("react/jsx-runtime");
const { create, act } = req("react-test-renderer");

const sent = [];
const listeners = [];
const fakeParent = {
  postMessage(msg) { sent.push(msg); },
};
const captured = { handoff: null };
const fakeWindow = {
  __ModuleLoader__: { load(h) { captured.handoff = h; } },
  parent: fakeParent,
  addEventListener(type, fn) { if (type === "message") listeners.push(fn); },
  removeEventListener() {},
  setInterval: () => 0,
  clearInterval: () => {},
};
new Function("window", clientJs)(fakeWindow);

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
const text = () => JSON.stringify(renderer.toJSON());

assert.ok(text().includes("正在连接鲸仔"), "未握手应显示连接中");
assert.ok(text().includes("鲸仔（Whalito）桌面端设置"), "应显示副标题");
assert.ok(text().includes("img") && text().includes("data:image/png;base64"), "应渲染应用 logo（data URI PNG）");
console.log("ok: 未握手状态（含应用 logo 与副标题）");

// 握手：父窗口推送 settings + status
const hello = {
  channel: "whalito",
  type: "hello",
  settings: {
    port: 3080, registry: "https://registry.npmjs.org", autostart: false,
    autoRestart: true, workspaceDir: null, nodeDir: null, petEnabled: true,
  },
  status: { phase: "running", url: "http://127.0.0.1:3080", pid: 1234 },
  versions: {
    dsh: { current: "0.2.1", latest: null, updateAvailable: false },
    whalito: { current: "0.2.0", testBuild: true, latest: null, updateAvailable: false, url: null },
  },
};
act(() => {
  listeners.forEach((fn) => fn({ data: hello, source: fakeParent }));
});
const t2 = text();
assert.ok(t2.includes("端口"), "应渲染端口字段");
assert.ok(t2.includes("npm 镜像源"), "应渲染镜像字段");
assert.ok(t2.includes("运行中"), "应显示服务器状态");
assert.ok(t2.includes("http://127.0.0.1:3080"), "应显示服务器地址");
assert.ok(!t2.includes("启动服务器"), "运行中不应显示启动按钮");
assert.ok(t2.includes("停止服务器"), "运行中应显示停止按钮");
assert.ok(t2.includes("显示桌宠"), "应渲染桌宠开关");
assert.ok(t2.includes("npmmirror（国内加速）"), "应渲染镜像源快捷切换");
assert.ok(t2.includes("版本信息"), "应渲染版本信息区块");
assert.ok(t2.includes("DSH：") && t2.includes("0.2.1"), "应显示 DSH 版本");
assert.ok(t2.includes("鲸仔：") && t2.includes("0.2.0（测试版）"), "应显示鲸仔版本与测试标记");
assert.ok(t2.includes("检查更新"), "应渲染检查更新按钮");
console.log("ok: 握手后表单完整渲染（含版本信息）");
assert.ok(sent.some((m) => m.type === "ping"), "挂载时应发送 ping");
console.log("ok: 挂载发送 ping");

// 镜像源快捷切换：点 npmmirror 预设 → 立即保存
const npmmirrorBtn = renderer.root.findAll(
  (n) =>
    n.type === "button" &&
    typeof n.props.children === "string" &&
    n.props.children.includes("npmmirror"),
)[0];
act(() => npmmirrorBtn.props.onClick());
const presetMsg = sent.find((m) => m.action === "save-settings");
assert.ok(presetMsg, "快捷切换应立即发送保存");
assert.equal(presetMsg.value.registry, "https://registry.npmmirror.com");
console.log("ok: npmmirror 快捷切换立即保存");

// 版本检查更新流程：点击 → 发送 check-update → 父窗口回传最新版本
// 点击后按钮文案变为"检查中…"，先缓存两个按钮的实例引用再逐个点击。
const checkBtnList = renderer.root.findAll(
  (n) => n.type === "button" && n.props.children === "检查更新",
);
assert.equal(checkBtnList.length, 2, "应有两个检查更新按钮");
const dshCheckBtn = checkBtnList[0];
const whalitoCheckBtn = checkBtnList[1];
act(() => dshCheckBtn.props.onClick());
assert.ok(
  sent.some((m) => m.action === "check-update" && m.target === "dsh"),
  "应发送 DSH 检查请求",
);
act(() => whalitoCheckBtn.props.onClick());
assert.ok(
  sent.some((m) => m.action === "check-update" && m.target === "whalito"),
  "应发送鲸仔检查请求",
);
act(() => {
  listeners.forEach((fn) =>
    fn({
      data: {
        channel: "whalito",
        type: "hello",
        settings: hello.settings,
        status: hello.status,
        versions: {
          dsh: { current: "0.2.1", latest: "0.3.0", updateAvailable: true },
          whalito: {
            current: "0.2.0",
            testBuild: true,
            latest: "0.2.5",
            updateAvailable: true,
            url: "https://github.com/entireyu/dsh-whalito-desk/releases/tag/v0.2.5",
          },
        },
      },
      source: fakeParent,
    }),
  );
});
const t3 = text();
assert.ok(t3.includes("发现新版本 0.3.0"), "DSH 应显示发现新版本");
assert.ok(t3.includes("更新请到鲸仔面板执行"), "DSH 应显示更新提示");
assert.ok(t3.includes("发现新版本 0.2.5"), "鲸仔应显示发现新版本");
const openBtn = renderer.root.findAll(
  (n) => n.type === "button" && n.props.children === "打开下载页",
)[0];
act(() => openBtn.props.onClick());
assert.ok(
  sent.some(
    (m) =>
      m.action === "open-url" &&
      m.url === "https://github.com/entireyu/dsh-whalito-desk/releases/tag/v0.2.5",
  ),
  "应发送打开下载页动作",
);
console.log("ok: 检查更新流程（DSH/鲸仔/打开下载页）");

// 立即更新：点击 → 发送 apply-update → 父窗口回传进度 → 显示进度文案
const applyBtn = renderer.root.findAll(
  (n) => n.type === "button" && n.props.children === "立即更新",
)[0];
act(() => applyBtn.props.onClick());
assert.ok(
  sent.some((m) => m.action === "apply-update"),
  "应发送立即更新动作",
);
act(() => {
  listeners.forEach((fn) =>
    fn({
      data: {
        channel: "whalito",
        type: "update-progress",
        message: "正在下载更新…",
      },
      source: fakeParent,
    }),
  );
});
assert.ok(text().includes("正在下载更新…"), "应显示更新进度文案");
console.log("ok: 立即更新流程");

// GitHub 按钮：点击 → 发送 open-url 指向项目主页
const githubBtn = renderer.root.findAll(
  (n) => n.type === "button" && n.props.children === "GitHub",
)[0];
act(() => githubBtn.props.onClick());
assert.ok(
  sent.some(
    (m) => m.action === "open-url" && m.url === "https://github.com/entireyu/dsh-whalito-desk",
  ),
  "GitHub 按钮应发送项目主页地址",
);
console.log("ok: GitHub 按钮");

// 编辑端口 + 保存
const rootNode = renderer.root;
const numberInput = rootNode.findAll(
  (n) => n.type === "input" && n.props.type === "number",
)[0];
act(() => numberInput.props.onInput({ target: { value: "8123" } }));
const saveBtn = rootNode.findAll(
  (n) => n.type === "button" && n.props.children === "保存设置",
)[0];
act(() => saveBtn.props.onClick());
const saveMsgs = sent.filter((m) => m.action === "save-settings");
const saveMsg = saveMsgs[saveMsgs.length - 1];
assert.ok(saveMsg, "保存应发送 save-settings");
assert.equal(saveMsg.value.port, 8123);
assert.equal(saveMsg.value.autoRestart, true);
assert.equal(saveMsg.value.petEnabled, true);
console.log("ok: 保存动作消息正确:", JSON.stringify(saveMsg.value));

// 非法端口：本地校验拦截，不发消息
act(() => numberInput.props.onInput({ target: { value: "99999" } }));
const before = sent.length;
act(() => saveBtn.props.onClick());
assert.equal(sent.length, before, "非法端口不应发送");
assert.ok(text().includes("端口必须是"), "应显示校验错误");
console.log("ok: 非法端口本地校验拦截");

// 父窗口错误推送 → 分区显示错误
act(() => {
  listeners.forEach((fn) => fn({ data: { channel: "whalito", type: "error", message: "保存失败：磁盘满" }, source: fakeParent }));
});
assert.ok(text().includes("保存失败：磁盘满"), "应显示父窗口错误");
console.log("ok: 错误消息显示");

console.log("interactive settings debug: all passed");
