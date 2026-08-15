# Changelog

本文件记录项目所有显著变更。

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循[语义化版本](https://semver.org/lang/zh-CN/)。

## [0.3.0] - 2026-08-16

### 新增
- DSH 设置面板内新增「鲸仔」分区：端口 / npm 镜像 / 开机自启 / 自动重启 / 工作目录 / Node 目录 / 桌宠开关，以及服务器启停 / 重启 / 返回鲸仔助手；与主窗口通过 postMessage 双向同步
- 鲸仔设置分区插件（@entireyu/whalito-dsh-settings，内嵌于应用）在启动服务器前幂等同步到 DSH web profile（node_modules + cordis.patch.yml 标记块），不依赖 pnpm，不动 DSH 源码

### 变更
- 移除内嵌页右下角悬浮按钮（fab）及独立 Webview 遗留代码（embed.rs / inject.js）；服务器未运行时内嵌页提供「启动服务器 / 返回鲸仔助手 / 打开设置」
- 在「鲸仔」分区保存端口变更后自动重启服务器生效
- 安装包名改为英文 Whalito（productName 变更，安装目录随之变为 %LOCALAPPDATA%\Whalito）
- 新增测试构建（pnpm tauri:build:test）：包名 Whalito-Test、标识符 com.deepseek.dsh-launcher.test、默认端口 30080、独立 DSH 数据目录 ~/.dsh-test，与生产包可共存
- 「鲸仔」设置分区握手加固：未连接时每 2 秒重试 ping、放宽 WebView2 的 event.source 校验、父窗口在未加载设置时也回握手
- 修复握手永远停留在「正在连接鲸仔…」的根因：postMessage 的 structured clone 不接受 Vue reactive Proxy，父窗口发送快照前改为 JSON 深拷贝（toPlain），并新增 %TEMP%\whalito-bridge.log 诊断通道
- 托盘图标悬浮提示显示应用名称「鲸仔 Whalito」，测试版末尾追加「（测试版）」
- 「鲸仔」设置分区更名为「鲸仔设置」，分区内容头部展示鲸仔应用 logo（64x64 PNG 以 data URI 内嵌，展示 32px/2x 密度），副标题「鲸仔（Whalito）桌面端设置」
- 服务器运行中不再显示「启动服务器」按钮（停止/重启仅运行时显示）
- npm 镜像源支持快速切换：一键切换 npmmirror（国内加速）/ npm 官方源并立即保存
- 「鲸仔设置」分区新增版本信息区块：分别展示 DSH 当前版本与鲸仔当前版本（测试版带标记），各自提供检查更新按钮——DSH 走 npm 镜像源检查，鲸仔走 GitHub releases（404 回退 tags）；鲸仔发现新版本附「打开下载页」；鲸仔行常驻「GitHub」按钮直达项目主页
- 鲸仔自动更新：分区内「立即更新」一键完成 下载 → 静默安装到当前目录 → 自动重启（安装包按变体自动选择 Whalito_/Whalito-Test_ 资产；下载经 Rust 直连，无浏览器 MOTW 标记，不会触发 SmartScreen 拦截）
- 桌宠右键菜单升级为与托盘同款：打开面板 / 启动服务器 / 停止服务器 / 在浏览器打开 / 隐藏桌宠 / 退出（菜单跟随鼠标位置并限制在桌宠窗口内）
- 桌宠修复与架构：服务器探测改为健康优先兜底链（记录地址 → 配置端口），API 失败显示具体原因并写 %TEMP%\whalito-pet.log 诊断日志；支持按住鲸仔拖拽窗口（4px 阈值区分点击）且位置持久化；点击桌宠不再重载内嵌页
- 桌宠样式 API：新增 ~/.dsh/pet-style.json 契约（尺寸/位置/头像/强调色/气泡配色/动画开关），变更 2 秒内热更新，Pet.vue 退化为默认渲染器；用户可经 DSH 编辑该文件调整外观（详见 README）
- 修复桌宠自上线以来一直显示「服务器未运行/正在连接」的根因：pet 窗口不在 Tauri capabilities 白名单，`plugin:event|listen` 被 ACL 拦截——已把 pet 加入白名单并授予 core:event:default 与窗口位置权限；拖拽改为手动移动窗口（缩放系数感知，4px 阈值区分点击），不再依赖透明窗口下不可靠的 startDragging

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

[0.3.0]: https://github.com/entireyu/dsh-whalito-desk/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/entireyu/dsh-whalito-desk/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/entireyu/dsh-whalito-desk/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/entireyu/dsh-whalito-desk/releases/tag/v0.1.0
