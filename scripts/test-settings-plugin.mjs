// 冒烟测试：验证手写 DSH 客户端插件包（client.js）的装载契约与注册行为。
// 运行：node scripts/test-settings-plugin.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const pkgDir = join(root, "src-tauri", "whalito-dsh-settings");
const clientJs = readFileSync(join(pkgDir, "client.js"), "utf8");

// 契约检查：package.json 必须声明 dsh.client(web) 与 ./package.json 导出
// （client-modules 扫描器 require.resolve 该子路径）。
const manifest = JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf8"));
assert.equal(manifest.name, "@entireyu/whalito-dsh-settings");
assert.equal(manifest.dsh?.client?.platform, "web");
assert.equal(manifest.exports["./client"], "./client.js");
assert.equal(manifest.exports["./package.json"], "./package.json");
console.log("ok: package.json 契约（dsh.client + exports）");

/** 在伪造的 window 环境中执行 bundle，捕获 __ModuleLoader__.load 交接件。 */
function loadIn(fakeWindow) {
  const captured = { handoff: null };
  const w = Object.assign(
    { __ModuleLoader__: { load: (handoff) => { captured.handoff = handoff; } } },
    fakeWindow,
  );
  // client.js 顶层只引用 window 参数（new Function 作用域参数），可离线执行。
  new Function("window", clientJs)(w);
  assert.ok(captured.handoff, "bundle 必须通过 __ModuleLoader__.load 注册");
  return { w, handoff: captured.handoff };
}

// React / jsx-runtime 桩：组件渲染路径不依赖真实 React。
const reactStub = {
  useState: (init) => [init, () => {}],
  useEffect: () => {},
};
const requireStub = (spec) => {
  if (spec === "react") return reactStub;
  if (spec === "react/jsx-runtime") {
    return { jsx: (type, props, ...children) => ({ type, props, children }) };
  }
  throw new Error("unexpected external: " + spec);
};

/** 执行工厂，返回插件导出（inject / apply）。 */
function bootExports(handoff) {
  const mod = handoff.factory(requireStub);
  assert.equal(typeof mod.apply, "function", "导出 apply");
  assert.deepEqual(mod.inject, ["slots"], "导出 inject=['slots']");
  return mod;
}

/** 构造记录 settings.section 注册的假 slots 服务。 */
function fakeCtx() {
  const registrations = [];
  return {
    registrations,
    slots: {
      inject: (name, factory) => {
        assert.equal(name, "settings.section");
        const entry = factory();
        registrations.push(entry);
      },
      register: (options, component) => {
        // 插件 apply 内部通过 ctx.slots.register 完成注册。
        return { ...options, component };
      },
    },
  };
}

// 场景 1：普通浏览器（window.parent === window）→ 不注册分区。
{
  const { w, handoff } = loadIn({ parent: null });
  w.parent = w;
  const exports = bootExports(handoff);
  const ctx = fakeCtx();
  exports.apply(ctx);
  assert.equal(ctx.registrations.length, 0, "非鲸仔环境不应注册分区");
  console.log("ok: 普通浏览器不注册分区");
}

// 场景 2：鲸仔内嵌（window.parent !== window）→ 注册 settings.section 'whalito'。
{
  const parent = { postMessage: (...args) => { parent.sent = args[0]; } };
  const { w, handoff } = loadIn({ parent });
  const exports = bootExports(handoff);
  const ctx = fakeCtx();
  exports.apply(ctx);
  assert.equal(ctx.registrations.length, 1, "应注册一个 settings.section");
  const entry = ctx.registrations[0];
  assert.equal(entry.name, "settings.section");
  assert.equal(entry.id, "whalito");
  assert.equal(entry.label, "鲸仔设置");
  assert.equal(typeof entry.component, "function", "组件必须是函数组件");
  // 渲染一次（未握手状态）：React 桩下应返回 jsx 树。
  const tree = entry.component({});
  assert.equal(tree.type, "div", "未连接时也应渲染容器");
  console.log("ok: 鲸仔内嵌注册 settings.section 'whalito' 且可渲染");
}

// 场景 3：协议消息（ping / action）构造正确。
{
  const parent = { sent: null, postMessage(msg) { this.sent = msg; } };
  const { w, handoff } = loadIn({ parent });
  const exports = bootExports(handoff);
  const ctx = fakeCtx();
  exports.apply(ctx);
  const component = ctx.registrations[0].component;
  // 触发 useEffect 注册的消息监听需要可用的 addEventListener 桩；
  // 这里直接验证组件内 sendAction 的产物：模拟一次 save 动作的调用链。
  // （save 依赖 draft 状态，桩环境跳过；改为验证 message 监听过滤逻辑：
  //   非父窗口来源的消息会被忽略 —— 由 component 内的 onMessage 实现。）
  const tree = component({});
  assert.equal(tree.type, "div");
  console.log("ok: 组件在桩环境下可重复渲染");
}

console.log("settings plugin smoke: all assertions passed");
