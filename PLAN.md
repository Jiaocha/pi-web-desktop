# Pi Web 桌面化 · 编程实施计划

> 目标：把 Pi Web（全栈 Next.js）以 Tauri v2 壳 + Node sidecar 的形式发布为 Windows 桌面应用，全流程在 GitHub Actions 构建。  
> 计划状态：**已确认并执行中**（D1–D4 已定）。M1 首个包（v0.12.0）启动失败已定位并修复，**v0.12.1 已发布**，待人工验收。

---

## 0. 当前基线（已确认的事实）

| 项        | 状态                                                                                                                                                                                   |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 仓库       | `Jiaocha/pi-web-desktop`（私有），本地副本 `pi-web-temp/desktop/`，工作区干净                                                                                                                       |
| 提交       | 7 次提交，最新 `20744a7`                                                                                                                                                                   |
| CI       | `Desktop CI` 最后一次 **success**（rust: `cargo fmt` + `cargo check`；payload: 载荷组装）                                                                                                       |
| 架构       | Tauri 壳 → 动态端口 → `node.exe bin/pi-web.js -H 127.0.0.1 -p <port> --no-open` sidecar → WebView2 指向 `http://127.0.0.1:<port>`（**pi-web CLI 没有 `start` 子命令**，裸 `pi-web [options]` 即启动服务） |
| 窗口       | `src-tauri/src/lib.rs:53` 已设为 `.decorations(false)`（无边框）→ **必须有拖拽区，否则窗口不可移动**                                                                                                        |
| 载荷       | `npm prune --omit=dev` 后 `app/` = **511.4MB**（`node_modules` 476.2MB）                                                                                                                |
| node-pty | 自带 `prebuilds/win32-x64/conpty.node`，普通安装即可用；runner 无 VS，**禁止** `--build-from-source`                                                                                                |
| 未填写      | `tauri.conf.json > plugins.updater.pubkey` 仍是占位符                                                                                                                                     |

**已验证的坑（不要再踩）**：NSIS 无 `createDesktopShortcut/createStartMenuShortcut`；capability `remote` 必须是 `{"urls":[...]}`；`externalBin` 与 `bundle.resources` 在编译期就校验；`tauri::Error` 不从 shell 插件错误转换。

---

## 1. 范围与非目标

**范围内**：Windows x64 可安装包、自动更新（draft 发布流程）、托盘与窗口生命周期、体积优化、CI 门禁。

**暂不做**：macOS/Linux、Windows ARM64（矩阵已留注释，后续开启）、把 API 层迁到 Rust、Bun 替换 Node（实测无体积收益）、Authenticode 代码签名（需证书采购）。

---

## 2. 阶段与子任务

### 阶段 0：前置决策（阻塞后续，需确认）

| ID | 决策                                    | 影响                           |
| -- | ------------------------------------- | ---------------------------- |
| D1 | pi-web 前端源码如何承载改动？（fork / 上游 PR / 不改） | 决定 T5 能否做、以及 `PI_WEB_REF` 指向 |
| D2 | 首个可安装版本是否允许「系统标题栏」临时方案                | 决定 T1 的实现方式与 M1 能否立刻出包       |
| D3 | 是否现在生成 updater 密钥并出第一个草稿包             | 决定 T3 何时执行                   |
| D4 | 是否接受 511MB 载荷先出包，还是先瘦身                | 决定 T8 的优先级                   |

---

### 阶段 1：M1 —— 可安装、可运行的最小版本

#### T1 · 定稿窗口形态与基础窗口控制

- **目标**：保证首个版本「窗口能用」——能移动、能最小化/最大化/关闭。
- **涉及文件**：`src-tauri/src/lib.rs`（`WebviewWindowBuilder`）、`src-tauri/capabilities/default.json`、可选新增 `src-tauri/src/window.rs`
- **依赖**：无
- **方案 A（零前端改动，推荐先做）**：`decorations(false)` → `decorations(true)`，使用系统标题栏；托盘保留。
- **方案 B（需 D1=可改前端）**：保持无边框，由前端提供标题栏 + `data-tauri-drag-region`。
- **补充（两种方案都做）**：`WindowEvent::CloseRequested` → `prevent_close()` + `hide()`，实现「关闭到托盘」，避免用户点 × 直接杀掉 sidecar 体验差。
- **验收**：CI 绿；打开安装包后窗口可拖动/最小化/关闭；点 × 后进程仍存活且可从托盘恢复；托盘「退出」后任务管理器无 `node.exe`。
- **风险**：方案 B 若前端未适配则窗口不可拖动（**阻塞性**）；`CloseRequested` 拦截后若托盘图标创建失败会导致窗口无法关闭 → 需保证托盘必建成功，否则回退到直接退出。

#### T2 · 应用图标

- **目标**：替换 CI 生成的占位图标，使安装包/开始菜单/托盘显示品牌图标。
- **涉及文件**：`app-icon.png`（新增，≥1024×1024）、`.gitignore`（移除 `/app-icon.png`）、`README.md`
- **依赖**：需用户提供品牌 PNG（或由我生成一个更正式的占位图）
- **验收**：CI 中 `npx tauri icon` 成功；安装后开始菜单快捷方式图标正确。
- **风险**：无图标资源时阻塞 → 可先用改进版占位图放行。

#### T3 · 签名密钥与首次出包

- **目标**：产出第一个可安装的 NSIS 草稿包。
- **涉及文件**：`src-tauri/tauri.conf.json`（填 `pubkey`）、仓库 Secrets（`TAURI_SIGNING_PRIVATE_KEY[_PASSWORD]`）、`.github/workflows/desktop-release.yml`
- **依赖**：T1、T2；三处 version 一致（`package.json` / `Cargo.toml` / `tauri.conf.json`）
- **步骤**：`npx tauri signer generate`（本机一次性，非构建）→ 填 pubkey → 存 Secrets → `git tag v0.12.0` → 推送触发发布。
- **验收**：Release 草稿含 `*.exe` + `*.sig` + `latest.json`，且 `latest.json` 中 `platforms.windows-x86_64.signature` 与 `.sig` 内容一致。
- **风险**：私钥丢失不可恢复（必须离线备份）；未填 pubkey 会导致更新校验失败，但**不影响首次安装**。

#### T4 · 人工验收（不写代码）

- **目标**：按验收清单验证首个安装包。
- **涉及文件**：`docs/desktop-tauri.zh-CN.md` 第 13 节清单
- **依赖**：T3
- **验收项**：冷启动无登录页；终端可用（含中文不乱码、resize）；SSE 与重连；文件浏览器读写；退出无残留 node；卸载后 `~/.pi/agent` 保留。
- **风险**：capability `remote` 通配符若不生效，前端窗口控制 API 会静默失败 → 验收时必点标题栏按钮验证；不生效则改为固定端口（回退方案：固定 31111 并写入 capability）。

---

### 阶段 2：M2 —— 体验完善

#### T5 · pi-web 侧适配（取决于 D1）

- **目标**：桌面端获得原生观感并隐藏浏览器专有 UI。
- **涉及仓库**：pi-web（fork 或上游 PR）；涉及 `app/layout.tsx`、新增 `components/DesktopTitleBar.tsx`、新增 `lib/tauri-bridge.ts`、PWA/登录相关组件
- **依赖**：D1；若采用无边框则与 T1 方案 B 联动（把 `decorations` 改回 `false`）
- **要求**：标题栏容器带 `data-tauri-drag-region`；窗口控制用 v2 API `getCurrentWindow()` + `LogicalSize`；退出走 `@tauri-apps/plugin-process`；读 `PI_WEB_DESKTOP=1` 隐藏 PWA 安装提示、LAN 提示、密码设置入口。
- **验收**：无边框窗口可拖动；三个按钮生效；桌面端不出现 PWA/密码设置入口；**浏览器端行为完全不变**（回归验证）。
- **风险**：污染上游行为（必须在 `PI_WEB_DESKTOP` 分支内，默认关闭）；`PI_WEB_REF` 需指向 fork，否则 CI 拉不到改动。

#### T6 · 单实例与启动健壮性

- **目标**：避免重复启动产生端口竞争与双份 sidecar。
- **涉及文件**：`src-tauri/Cargo.toml`（加 `tauri-plugin-single-instance`）、`src-tauri/src/lib.rs`、`src-tauri/src/server.rs`（端口占用重试）
- **依赖**：T1
- **验收**：二次启动聚焦已有窗口；sidecar 端口被占时能换端口重试并成功启动。
- **风险**：单实例插件与 `RunEvent::Exit` 清理逻辑的交互，需确认第二次进程的退出不误杀第一个实例的 sidecar。

#### T7 · 日志落盘

- **目标**：sidecar 与壳的日志可排查（现在是 `eprintln!`，release 构建下无控制台）。
- **涉及文件**：新增 `tauri-plugin-log`（写入 `%LOCALAPPDATA%`）、`src-tauri/src/server.rs`（去掉 `eprintln!`，改 log 宏）、`capabilities/default.json`
- **依赖**：T1
- **验收**：安装版运行后可在日志目录找到 sidecar 输出；启动失败时有可读错误。
- **风险**：日志文件无限增长 → 需配置轮转或大小上限。

---

### 阶段 3：M3 —— 体积

#### T8 · standalone 裁剪

- **目标**：把 `app/` 从 511MB 降到 ~120–180MB。
- **涉及仓库/文件**：pi-web 的 `next.config.ts`（`output: "standalone"` + `outputFileTracingIncludes` 补回 node-pty/undici/web-push/@earendil-works/*）；桌面侧 `scripts/prepare-desktop.mjs`（支持拷贝 standalone 目录）、新增 `app/desktop-server.mjs`（调用 `bin/rotate-preview-secrets.js` 后启动 standalone server）
- **依赖**：D1（需改 pi-web 的 next.config.ts）；T4（先确认功能版本可用，避免瘦身引入新变量）
- **验收**：CI 的 `du -sh app` 显著下降；安装包体积下降；终端、SSE、会话读写仍全部正常（重点回归 node-pty 是否被追踪进 standalone）。
- **风险**：`serverExternalPackages` 不会被自动追踪 → 漏拷会导致终端打不开（脚本必须断言 `prebuilds/<os>-<arch>` 存在）；绕过 `bin/pi-web.js` 会丢失 preview secret 轮换，必须在启动器里补回。

#### T9 · 体积门禁

- **目标**：防止体积回归。
- **涉及文件**：`.github/workflows/desktop-ci.yml`（解析 `du` 输出，超过阈值则失败/告警）
- **依赖**：T8
- **验收**：人为把阈值调低时 CI 失败；正常值下通过。
- **风险**：阈值设太紧会频繁误报 → 先设告警级别。

---

### 阶段 4：M4 —— 分发

#### T10 · 更新链路端到端验证

- **目标**：确认旧版能自动升级且数据不丢。
- **涉及文件**：版本号三处、tag 发布流程
- **依赖**：T3（至少两个版本）
- **验收**：装 v0.12.0 → 发布 v0.12.1 → 旧版弹出更新 → 安装后版本号变更、`~/.pi/agent` 会话不丢。
- **风险**：Windows 安装器会强制退出应用（已用 `on_before_exit`/relaunch 处理）；`latest.json` 必须与签名一致。

#### T11 · 代码签名（可选）

- **目标**：消除 SmartScreen 警告。
- **涉及文件**：`.github/workflows/desktop-release.yml`（`signCommand` 或证书环境）
- **依赖**：证书采购（成本）
- **风险**：与 updater 的 `.sig` 是两套独立签名，别混淆。

#### T12 · 多架构/多平台（可选）

- **目标**：Windows ARM64、macOS。
- **涉及文件**：`desktop-release.yml` matrix（注释已备好）、`prepare-desktop.mjs`（已支持 darwin/linux 的 tar.gz 提取）
- **依赖**：T10；macOS 还需签名/notarize
- **风险**：`PI_WEB_REF` 与 node-pty 的 macOS 预编译（1.2.0-beta.15 的 macOS 预编译可能仍需源码构建 → runner 无 Xcode 命令行工具？需实测）。

---

### 阶段 5：便携版（绿色免安装，新增需求）

#### T13 · portable zip（解压即用）

- **目标**：产出单个 zip，解压后双击即可运行；不安装、不写注册表、不需要管理员权限，运行时数据全部留在解压目录内，可放 U 盘。
- **涉及文件**：新增 `src-tauri/src/portable.rs`、`scripts/make-portable.mjs`；改 `src-tauri/src/{lib.rs,logging.rs,server.rs}`、`.github/workflows/desktop-release.yml`、`package.json`、`.gitignore`、`README.md`
- **依赖**：T3（复用已编译的 release exe，因此**不增加第二次 Rust 编译**）
- **实现要点**
  - `portable.txt` 标记文件放在 exe 同级 → 便携模式开关；无标记时行为与安装版完全一致（零回归风险）
  - 三处路径重定向：日志 → `<root>/data/logs`；WebView2 profile → `<root>/data/webview`（`WebviewWindowBuilder::data_directory`）；pi-web 状态 → `<root>/data/pi/agent`（sidecar 传 `PI_CODING_AGENT_DIR`，实测 pi-web 全部状态都在 agent 目录下）
  - `app_dir()` 增加回退：先查 Tauri resource dir，再查 exe 同级 `app/`
  - 便携版**禁用自动更新**（更新走 NSIS 会写 Program Files）
  - 附带 `vcruntime140*.dll`（实测 Rust exe 只依赖 UCRT，node.exe 自带，附带仅为兜底）、`runtime/` WebView2 引导器（best-effort 下载）、`README.txt`、`run-debug.bat`、`portable.txt`
  - 压缩用 7z，缺 7z 回退 `tar -a`
- **验收**：CI 绿；zip 上传到同一 Release；解压后双击 exe 出窗口；`data/` 下出现 `logs`、`webview`、`pi/agent`；把整个目录复制到别处（含 U 盘）后数据跟随；进程退出无残留 `node.exe`
- **风险**
  - WebView2 缺失的机器上窗口打不开 → 已在 `runtime/` 提供安装器与说明（Win10 21H2+/Win11 自带，绝大多数机器不受影响）
  - 未代码签名 → SmartScreen 会拦截，需「更多信息 → 仍要运行」
  - zip 体积较大（app 载荷 511MB，压缩后约 150–200MB）→ 由 T8 瘦身解决
  - `PI_CODING_AGENT_DIR` 若被上游改名，便携化会失效 → `scripts/prepare-desktop.mjs` 未做断言，需在验收时确认 `data/pi/agent` 真的生成

---

## 3. 子任务总表（依赖与并行）

| ID      | 任务            | 依赖         | 可并行           |
| ------- | ------------- | ---------- | ------------- |
| T1      | 窗口形态 + 关闭到托盘  | —          | 与 T2 并行       |
| T2      | 图标            | 品牌图        | 与 T1 并行       |
| T3      | 签名密钥 + 首次出包   | T1、T2      | —             |
| T4      | 人工验收          | T3         | —             |
| T5      | pi-web 侧适配    | D1、T1(方案B) | 与 T6/T7 并行    |
| T6      | 单实例 + 端口重试    | T1         | 与 T5/T7 并行    |
| T7      | 日志落盘          | T1         | 与 T5/T6 并行    |
| T8      | standalone 裁剪 | D1、T4      | —             |
| T9      | 体积门禁          | T8         | —             |
| T10     | 更新链路验证        | T3 + 第二个版本 | —             |
| T11/T12 | 代码签名 / 多平台    | T10        | 可选，最后         |
| T13     | 便携版 zip       | T3（复用 exe） | 与 T5/T6/T7 并行 |

## 4. 推荐执行顺序

1. **确认 D1–D4**（阻塞一切）
2. **并行批次 A**：T1（窗口）+ T2（图标）→ 一次 CI 验证
3. **T3** 生成密钥、填 pubkey、打 tag → 出首个草稿包
4. **T4** 人工验收（我给出清单，你执行安装验证并反馈结果）
5. **并行批次 B**：T5（前端适配）+ T6（单实例）+ T7（日志）
6. **T8** standalone 瘦身 → **T9** 体积门禁
7. **T10** 更新链路验证 → 之后按需 T11/T12

---

## 5. 需要确认的问题

> 已确认（2026-10-12）：D1 = Fork 到 `Jiaocha/pi-web`（已建，fork 自 agegr/pi-web，MIT）；D2 = 方案 A 系统标题栏；D3 = T1+T2 后立刻出包；D4 = 先出包再瘦身。

---

## 6. 执行记录

| 任务      | 状态    | 改动                                                                                                                                                                                                                        | 结果                                                                                                                                                                                                                                        |
| ------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1 fork | ✅     | `Jiaocha/pi-web`（公开 fork，默认分支 main）；两个工作流的 `PI_WEB_REF` 已切换                                                                                                                                                               | `gh repo fork` 成功                                                                                                                                                                                                                         |
| T1 窗口形态 | ✅     | `src-tauri/src/lib.rs`：`decorations(true)`（系统标题栏，M1 零前端依赖）；新增 `on_window_event(CloseRequested)` → `prevent_close()` + `hide()`（关闭到托盘，托盘「退出」才是真退出）                                                                         | CI `cargo check` ✅                                                                                                                                                                                                                        |
| T2 图标   | ✅（占位） | `scripts/make-icon.mjs`：改为 π 字形白图 + #2563EB 圆角方底；本地生成并通过 PNG 结构校验                                                                                                                                                         | 见 issue #1（待品牌图替换）                                                                                                                                                                                                                        |
| T3 密钥   | ✅     | `tauri signer generate`（本机一次性；私钥在 `~/.tauri/pi-web-desktop.key`，无密码）；pubkey 已填入 `tauri.conf.json`；`TAURI_SIGNING_PRIVATE_KEY` 已写入仓库 Secret                                                                                | `gh secret list` 确认                                                                                                                                                                                                                       |
| T3 首次出包 | ✅     | `git tag v0.12.0` → `Desktop release` 24 分钟跑完                                                                                                                                                                             | 草稿 Release 含：`Pi.Web.Desktop_0.12.0_x64-setup.exe`（**98.7MB**）、`.sig`、`latest.json`；**latest.json 签名与 .sig 完全一致**（已脚本比对）                                                                                                                  |
| T8 载荷瘦身 | ⬜     | 未开始（先出包再瘦身）                                                                                                                                                                                                               | 实测安装包 98.7MB，在 70–120MB 预估区间内；瘦身目标改为缩小安装后占用与更新下载量                                                                                                                                                                                         |
| T4 人工验收 | ❌→🔧  | —                                                                                                                                                                                                                         | **首次安装后无法启动**。本机复现 `pi-web-desktop.exe` 拿到 sidecar stderr：`Error: EISDIR ... lstat 'D:'`；手工跑 `node bin/pi-web.js start` → `Unexpected argument(s): start`。**根因：pi-web CLI 无 `start` 子命令**，node 启动即退出 1，健康检查白等 30s，release 构建下错误无处可见（静默退出） |
| 启动修复    | ✅     | `src-tauri/src/server.rs`：参数改为 `[entry, -H, 127.0.0.1, -p <port>, --no-open]`（去掉 `start`；`--no-open` 同时阻止 CLI 再开系统浏览器）；entry 存在性校验；记录 app dir/entry/port                                                                  | 本机验证：修正后的调用 `✓ Ready in 364ms`，`curl -I` 返回 **200**                                                                                                                                                                                       |
| 失败可诊断   | ✅     | 新增 `src-tauri/src/logging.rs`：`ServerLog` 环形缓冲（500 行）+ 落盘 `<app data>/logs/desktop.log`；`lib.rs` 新增 `boot()` / `report_failure()`：失败时写 `logs/startup-failure.txt`、用记事本打开并打开日志目录；`wait_until_ready` 子进程提前退出即带上退出码与输出尾部快速失败 | 静默退出问题消除                                                                                                                                                                                                                                  |


| 开发者模式 | ✅ | `tray.rs` 托盘新增「开发者工具」（`window.open_devtools()`）与「打开日志目录」；`Cargo.toml` 启用 tauri `devtools` feature（release 也可用）；`PI_WEB_DESKTOP_DEBUG=1` 自动开 DevTools；`PI_WEB_DESKTOP=1` 供前端识别桌面端；`README.md` 新增「开发者模式」章节 | CI `rust`(fmt+check) ✅、`payload` ✅（run 38187756598） |  
| 开发者模式 + | ✅ | `PI_WEB_DESKTOP_DEV_URL=<url>`：**跳过 sidecar** 直连本地服务，窗口标题带 ` (dev)`；仍 manage 空的 `ServerHandle`，保证 `RunEvent::Exit` 与托盘正常 | CI `rust`+`payload` ✅（run 38189827712）；前端迭代无需重组载荷/重装 |  
| v0.12.1 发布 | ✅ | 三处版本 0.12.0 → 0.12.1，tag `v0.12.1` 触发 `Desktop release`（22m2s） | **已发布**（非草稿）：`Pi.Web.Desktop_0.12.1_x64-setup.exe`（103,545,130 B）+ `.sig` + `latest.json`，签名比对 **True** |
| T13 便携版 | ✅ | 新增 `src-tauri/src/portable.rs`（`portable.txt` 标记 → `data_dir()`）+ `scripts/make-portable.mjs`；`logging.rs` 日志改 `<root>/data/logs`；`lib.rs` 用 `WebviewWindowBuilder::data_directory(<root>/data/webview)`、便携版跳过 updater；`server.rs` 传 `PI_CODING_AGENT_DIR=<root>/data/pi/agent`；发布工作流增加「组装 zip → 报告依赖 → 上传 Release」 | v0.13.0 发布：`Pi-Web-Desktop-0.13.0-portable-win64.zip`（**179.7 MB**，解压 602.5 MB，29982 条目）；实测两个 exe 的 VC/CRT imports 均为 **none** |
| T13 踩坑① | ✅ | `sizeOf()` 对 zip 文件调 `readdirSync` → `ENOTDIR`（zip 其实已打好，只差最后一行日志） | 改为 `fs.statSync(zipFile).size` |
| 启动修复② | ✅ | **根因：Tauri `path().resolve()` 返回 `\\?\` 规范化路径，作为程序名传给 `CreateProcessW` 被拒（os error 2）**。`server.rs` 改为自己解析 node 运行时与 payload、剥掉 `\\?\` 前缀，用 `std::process::Command` 启动（Windows 加 `CREATE_NO_WINDOW`）；退出检测改用 `Child::try_wait()`；stdout/stderr 用线程泵进日志环；移除 `tauri-plugin-shell` 与对应 capability | 本机实测：带 `\\?\` spawn → `[WinError 2] 系统找不到指定的文件。`（与现场报错逐字一致），去掉前缀即成功。本地 release exe 放进真实便携目录：`✓ Ready in ~340ms`，8→64s 全程 `HTTP 200`；`data/{logs,webview,pi/agent}` 全在解压目录内 |
| 构建修复③ | ✅ | `schemars 0.8.22` 声明 `indexmap 1.x` 但没开 `std` feature → `IndexMap` 的 `S` 无默认类型 → `tauri-build` 编译失败。**必须写在 `[build-dependencies]`**（resolver v2 下构建依赖特性独立解析）；同时提交 `Cargo.lock` 防止 CI 再解析到坏组合 | 本地与 CI（run 38200856766）均通过 |
| 本地构建流程 | ✅ | Rust 1.96 可用：`mkdir app` + 拷 `src-tauri/binaries/node-<triple>.exe` + `npm install` + `node scripts/make-icon.mjs` + `npx tauri icon` + `cargo build --release`（7m19s）。crates.io 慢时需 `CARGO_HTTP_LOW_SPEED_LIMIT=0 CARGO_HTTP_TIMEOUT=600` | **不消耗 GitHub 额度**即可产出可用 exe，已直接替换 `D:\IDM\压缩\...` 里的 exe |

---

## 7. 遗留问题与下一步

1. **T4 人工验收（重做，装 v0.12.1）**：装 `Pi.Web.Desktop_0.12.1_x64-setup.exe` 验证——冷启动无登录页、终端中文/resize、SSE 流式、标题栏按钮（验证 capability `remote` 生效）、退出无残留 node、卸载后 `~/.pi/agent` 保留。启动失败时先看 `%LOCALAPPDATA%\com.earendil.pi-web\logs\startup-failure.txt`。**旧的 v0.12.0 草稿（启动有问题的包）建议删除**，避免误装。
2. **T8 standalone 瘦身**（issue #3）：载荷 511MB → 120–180MB；注意 `serverExternalPackages` 需 `outputFileTracingIncludes` 补回。
3. **T5 前端适配**（issue #2）：`PI_WEB_DESKTOP=1` 隐藏浏览器 UI + 无边框标题栏（`data-tauri-drag-region`），改完把 `lib.rs` 的 `decorations` 切回 `false`。
4. **T6/T7**：单实例插件、`tauri-plugin-log` 落盘日志。
5. **issue #1**：品牌图标替换占位 π 图。
6. 私钥 `~/.tauri/pi-web-desktop.key` **离线备份**（丢失后已装用户无法再更新）。
7. **pi-web 前端源码**：fork 到 `Jiaocha/pi-web` 并让 CI 的 `PI_WEB_REF` 指向它？还是只改上游仓库本地、给上游提 PR？或第一阶段完全不改前端？
8. **首个版本窗口形态**：方案 A（系统标题栏，零前端改动，立刻可出包）还是方案 B（无边框 + 前端标题栏，需先做 T5）？
9. **出包时机**：现在就生成签名密钥并打 `v0.12.0` 出草稿包，还是等 T5/T8 完成后再出？
10. **体积策略**：接受 511MB 载荷先出包验证功能，还是先做完 standalone 瘦身？
