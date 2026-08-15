// 调试：用真实 React（来自 dsh 安装）渲染 whalito 设置分区组件，
// 复现"点击鲸仔分区一片空白"并打印具体错误。
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const clientJs = readFileSync(join(root, "src-tauri", "whalito-dsh-settings", "client.js"), "utf8");

// 真实 react 来自 dsh 安装包（与线上 web 页面共享同一份依赖树）。
const req = createRequire("C:/Users/EvanH/AppData/Local/dsh-launcher/npm/node_modules/@deepseek-ai/dsh/package.json");
const React = req("react");
const jsxRuntime = req("react/jsx-runtime");
const { renderToString } = req("react-dom/server");

const sent = [];
const fakeParent = { postMessage(msg) { sent.push(msg); } };
const captured = { handoff: null };
const fakeWindow = {
  __ModuleLoader__: { load(h) { captured.handoff = h; } },
  parent: fakeParent,
  addEventListener() {},
  removeEventListener() {},
};
// 顶层脚本只引用 window 参数
new Function("window", clientJs)(fakeWindow);
if (!captured.handoff) throw new Error("no handoff");

const requireStub = (spec) => {
  if (spec === "react") return React;
  if (spec === "react/jsx-runtime") return jsxRuntime;
  throw new Error("unexpected external: " + spec);
};
const mod = captured.handoff.factory(requireStub);

const registrations = [];
const ctx = {
  slots: {
    inject(name, factory) { const e = factory(); registrations.push(e); },
    register(options, component) { return { ...options, component }; },
  },
};
mod.apply(ctx);
console.log("registrations:", registrations.length);
const entry = registrations[0];
console.log("entry options:", JSON.stringify({ id: entry.id, order: entry.order, label: entry.label }));

// 1) 未握手状态渲染
try {
  const html1 = renderToString(jsxRuntime.jsx(entry.component, {}));
  console.log("--- unconnected render ---");
  console.log(html1);
} catch (e) {
  console.log("UNCONNECTED RENDER ERROR:", e && e.stack ? e.stack : e);
}

// 2) 模拟握手后状态：注入 settings/status 再渲染一次（组件内部 useState 是真实 React，
//    这里用两个独立 renderToString 各走一遍初始态；握手指的是 effect 里收消息，
//    renderToString 不跑 effect，所以直接构造 connected 初始态不可行——改用第二次
//    renderToString 并在同一 React 实例上不行。退而求其次：验证表单路径 = 手动调用
//    component 内的 save 不可行。先只验证首屏。
console.log("sent messages:", JSON.stringify(sent));
