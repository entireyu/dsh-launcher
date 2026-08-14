# Changelog

本文件记录项目所有显著变更。

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循[语义化版本](https://semver.org/lang/zh-CN/)。

## [0.2.0] - 2026-08-14

### 新增
- 引导式主流程：启动即进入 loading，自动检测 Node/dsh/服务器状态，按状态进入「装 Node / 装 dsh / 启动服务器」并最终在应用内嵌打开 Harness 页面
- 内嵌浏览器打开：新增独立 Webview 窗口加载 dsh 页面，注入悬浮按钮（返回助手 / 启动 / 停止 / 重启 / 设置），动作通过虚拟主机名导航 + `on_navigation` 拦截实现，不开放远程 IPC
- Node 版本门槛：要求 Node ≥ 22.19.0，缺失或过低统一进入安装引导
- 安装 Node 三种方式：nvm 安装切换、winget 一键（安装/升级）、自定义目录便携版（下载 zip 解压）
- 服务器启动改为「等待就绪」（轮询健康检查，超时报错），不再依赖 stdout URL 抽取
- 服务器一键重启
- 单实例运行：二次启动时唤起已有窗口
- Harness 版本更新检查：自动比对最新版本并提示可更新 / 已最新
- 托盘状态同步、设置面板改为弹窗、安装 / 更新 / 校验进度提示

### 变更
- 品牌重构：应用更名「鲸仔（Whalito）」，更新 logo 与全套图标，二进制名改为 Whalito

### 修复
- 校验安装改用 `dsh web --dump-default-config`，修复 `--profile` 报错

## [0.1.1] - 2026-08-14

### 修复
- 修复残缺安装误判与启动失败自动重试死循环

## [0.1.0] - 2026-08-14

### 新增
- 环境检测：自动检测 Node.js / npm / `@deepseek-ai/dsh` 是否就绪（含 nvm、winget 等常见位置与 `Program Files` 兜底）
- 一键安装 Node.js（`winget install OpenJS.NodeJS.LTS`）
- 一键安装 / 更新 Harness（`npm install -g @deepseek-ai/dsh`，可切换 npm 镜像源）
- 安装校验（`dsh --version` + `--dump-default-config`）
- 服务器管理：启动 / 停止 `dsh web --port <port>`，实时状态与 HTTP 健康检查
- 托盘常驻、开机自启、异常自动重启
- 实时日志回显
- npm 前缀兜底、托盘菜单状态联动、自定义图标
- 支持停止外部启动的 DSH 服务器（按端口定位进程 + 二次确认）

### 修复
- 安装 / 启动等阻塞命令改为异步，修复点击安装后界面卡死
- 探测端口上外部已运行的 DSH 服务器，修复误报已停止

[0.2.0]: https://github.com/entireyu/dsh-whalito-desk/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/entireyu/dsh-whalito-desk/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/entireyu/dsh-whalito-desk/releases/tag/v0.1.0
