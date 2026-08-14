// DeepSeek Harness 助手 —— 内嵌页面悬浮按钮
// 通过 initialization_script 在每次文档创建时注入（主 frame）。
// 按钮动作通过导航到虚拟主机名 dshlauncher.local 触发，由 Rust on_navigation 拦截。
(function () {
  if (window.__dsh_launcher_injected__) return;
  window.__dsh_launcher_injected__ = true;

  var PREFIX = '__dsh_launcher_';
  var ACTION_HOST = 'dshlauncher.local';

  function trigger(action) {
    try {
      window.location.href = 'http://' + ACTION_HOST + '/' + action;
    } catch (e) {
      /* ignore */
    }
  }

  function build() {
    if (document.getElementById(PREFIX + 'root')) return;

    var root = document.createElement('div');
    root.id = PREFIX + 'root';
    root.style.cssText =
      'position:fixed;right:20px;bottom:20px;z-index:2147483000;font-family:"Segoe UI","Microsoft YaHei",system-ui,sans-serif;';

    var menu = document.createElement('div');
    menu.id = PREFIX + 'menu';
    menu.style.cssText =
      'display:none;flex-direction:column;gap:4px;margin-bottom:8px;background:#171a21;' +
      'border:1px solid #2a2f3a;border-radius:10px;padding:6px;box-shadow:0 8px 24px rgba(0,0,0,.4);';

    function item(text, action) {
      var b = document.createElement('button');
      b.textContent = text;
      b.style.cssText =
        'background:#1e222b;color:#e8eaf0;border:1px solid #2a2f3a;border-radius:6px;' +
        'padding:6px 12px;cursor:pointer;font-size:13px;text-align:left;white-space:nowrap;';
      b.onmouseover = function () { b.style.borderColor = '#4f8cff'; };
      b.onmouseout = function () { b.style.borderColor = '#2a2f3a'; };
      b.onclick = function (e) {
        e.stopPropagation();
        trigger(action);
      };
      return b;
    }

    menu.appendChild(item('返回助手', 'focus-main'));
    menu.appendChild(item('启动服务器', 'start-server'));
    menu.appendChild(item('停止服务器', 'stop-server'));
    menu.appendChild(item('重启服务器', 'restart-server'));
    menu.appendChild(item('打开设置', 'open-settings'));

    var btn = document.createElement('button');
    btn.id = PREFIX + 'btn';
    btn.textContent = '\u2699';
    btn.title = 'DeepSeek Harness 助手';
    btn.style.cssText =
      'display:block;margin-left:auto;width:44px;height:44px;border-radius:50%;background:#4f8cff;' +
      'color:#fff;border:none;cursor:pointer;font-size:20px;box-shadow:0 4px 14px rgba(79,140,255,.5);';
    btn.onclick = function (e) {
      e.stopPropagation();
      menu.style.display = menu.style.display === 'none' ? 'flex' : 'none';
    };

    root.appendChild(menu);
    root.appendChild(btn);
    (document.body || document.documentElement).appendChild(root);

    document.addEventListener(
      'click',
      function (e) {
        if (!root.contains(e.target)) menu.style.display = 'none';
      },
      true
    );
  }

  function ensure() {
    if (!document.getElementById(PREFIX + 'root')) build();
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', ensure);
  } else {
    ensure();
  }

  // SPA 重渲染后按钮可能被移除，观察 DOM 变化 + 定时兜底重新挂载。
  new MutationObserver(function () {
    if (!document.getElementById(PREFIX + 'root')) ensure();
  }).observe(document.documentElement, { childList: true, subtree: true });
  setInterval(ensure, 2000);
})();
