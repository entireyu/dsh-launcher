# Changelog

本文件记录项目所有显著变更。

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循[语义化版本](https://semver.org/lang/zh-CN/)。

## [未发布]

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

[未发布]: https://github.com/entireyu/dsh-launcher/compare/v0.1.1...main
[0.1.1]: https://github.com/entireyu/dsh-launcher/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/entireyu/dsh-launcher/releases/tag/v0.1.0
