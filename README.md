# DeepSeek Harness 助手 (dsh-launcher)

面向小白用户的 DeepSeek Harness 一键安装与启动工具。Windows 优先，基于 **Tauri 2 + Vue 3 + TypeScript**。

## 功能

- **环境检测**：自动检测 Node.js / npm / `@deepseek-ai/dsh` 是否就绪（含 nvm、winget 等常见安装位置与 `Program Files` 兜底）。
- **一键安装 Node.js**：检测到缺失时通过 `winget install OpenJS.NodeJS.LTS` 静默安装。
- **一键安装 / 更新 Harness**：`npm install -g @deepseek-ai/dsh`，可切换 npm 镜像源（国内可改 `https://registry.npmmirror.com`）。
- **安装校验**：通过 `dsh --version` + `dsh --dump-default-config` 不占端口地验证是否安装成功。
- **服务器管理**：以子进程方式启动 `dsh web --port <port>`，解析 stdout 里的真实 URL，HTTP 健康检查实时反馈「已停止 / 启动中 / 运行中 / 异常」，停止时用 `taskkill /T /F` 清理整棵进程树。
- **托盘常驻 + 开机自启**：关闭窗口最小化到托盘；可开机自启（写入 HKCU Run 键）；服务器异常退出可自动重启。
- **实时日志**：后端 stdout/stderr 实时流式回显到界面。

## 目录结构

```
src/                    # Vue 3 + TS 前端控制面板
src-tauri/
  src/
    lib.rs              # 入口：托盘、窗口、命令注册
    state.rs            # 共享状态、配置、进程/日志/健康检查工具
    commands.rs         # 全部 Tauri 命令（检测/安装/启停/设置）
  Cargo.toml
  tauri.conf.json
```

## 开发

```bash
pnpm install
pnpm tauri dev        # 开发调试（前端 dev server + Rust debug）
```

## 打包

```bash
pnpm tauri build      # 产出 NSIS 安装器（src-tauri/target/release/bundle/nsis/）
```

> 安装包当前**未签名**，Windows SmartScreen 会提示「未知发布者」，属预期；正式分发建议配置代码签名证书。

## 原理：程序背后的真实命令

| 动作 | 实际执行 |
|---|---|
| 检测 Node | `where.exe node` / `node --version` / 兜底 `C:\Program Files\nodejs\node.exe` |
| 装 Node | `winget install OpenJS.NodeJS.LTS --silent --accept-package-agreements --accept-source-agreements` |
| 装/更新 Harness | `node <npm-cli.js> install -g @deepseek-ai/dsh[ @latest]` |
| 校验 | `node <dsh/bin.js> --version` + `--dump-default-config` |
| 启动 | `node <dsh/bin.js> web --port <port>` |
| 停止 | `taskkill /PID <pid> /T /F` |
| 健康检查 | `GET <解析出的 URL>`（800ms 超时） |
