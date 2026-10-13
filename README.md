# Pi Web Desktop

把 [Pi Web](https://github.com/agegr/pi-web)（全栈 Next.js 应用）打包成 Windows 桌面应用：
Tauri v2 只做窗口壳与生命周期，真正的服务由内置的 Node sidecar 运行
`bin/pi-web.js -H 127.0.0.1 -p <动态端口> --no-open`，WebView2 指向 `http://127.0.0.1:<动态端口>`。
（注意：pi-web CLI **没有 `start` 子命令**，裸 `pi-web [options]` 才是启动服务。）

## 架构要点

- **不能**用 `frontendDist` 直接指向 `.next`：Pi Web 有 40+ 个 Route Handler、SSR 与 SSE，
  必须带 JS 运行时。因此采用 sidecar 模型。
- 文件读写、终端（node-pty）、git、MCP **全部留在 Node 侧**，前端不授予 fs / shell 权限，
  因此浏览器版与桌面版行为完全一致。
- 退出时 `RunEvent::Exit` 会 kill sidecar，不会残留 `node.exe`。

## 目录

```
.github/workflows/   CI：PR 校验 + tag 发布
scripts/             载荷组装（下载 Node sidecar、拷贝 .next/node_modules、校验 node-pty）
                     与占位图标生成
src-tauri/           Tauri 壳：Cargo.toml / tauri.conf.json / capabilities / Rust 源码
src-app/             本地开发时 clone 的 Pi Web 源码（CI 中自动 checkout，不入库）
```

## 本地开发

```bash
# 1. 准备 Pi Web 源码（CI 不需要这一步，它会自动 checkout）
git clone https://github.com/agegr/pi-web src-app
# 或者：git submodule add https://github.com/agegr/pi-web src-app

# 2. 安装依赖
npm install          # 根目录：@tauri-apps/cli
npm --prefix src-app ci

# 3. 生成图标（仓库不提交二进制；有真实品牌图时替换 app-icon.png）
node scripts/make-icon.mjs app-icon.png
npx tauri icon app-icon.png -o src-tauri/icons

# 4. 启动：Tauri 壳 + src-app 的 next dev（30141）
npm run dev
```

本地**不做** release 构建（`cargo build --release` / `tauri build`）——交给 GitHub Actions。
本机只需 `cargo check`（在 `src-tauri` 目录下）与编辑器。

## 便携版（绿色免安装）

`Desktop release` 在产出 NSIS 安装包的同时，复用同一个已编译的 exe 组装一份 **zip 便携版**：
解压即可运行，不安装、不写注册表、不需要管理员权限。

```
Pi-Web-Desktop-<version>-portable-win64/
├─ Pi Web Desktop.exe            双击启动
├─ run-debug.bat                 排障入口：开 devtools、保留控制台、打印退出码
├─ node-x86_64-pc-windows-msvc.exe   内置 Node 运行时（按此名查找，勿改名）
├─ vcruntime140*.dll             微软 C 运行库（没装 VC++ 运行库的机器也能直接跑）
├─ app\                          Pi Web 本体（Next.js 产物 + node_modules + bin）
├─ data\                         运行时全部写入这里：logs / webview / pi\agent
├─ runtime\                      可选：老机器用的 WebView2 安装器
├─ portable.txt                  便携模式开关（删掉就退回 %APPDATA%）
└─ README.txt                    简短说明
```

**便携模式怎么实现的**：exe 同级有 `portable.txt` 时，外壳把三处路径全部改写到解压目录内——

| 原来 | 便携模式 |
|---|---|
| `%APPDATA%\…\logs` | `<root>\data\logs` |
| WebView2 profile（默认在用户目录） | `<root>\data\webview`（`WebviewWindowBuilder::data_directory`） |
| `~/.pi\agent`（会话、设置、run records） | `<root>\data\pi\agent`（给 sidecar 传 `PI_CODING_AGENT_DIR`） |

同时便携版**禁用自动更新**（更新要走安装包，会写到 Program Files）。

组装脚本是 `scripts/make-portable.mjs`（`npm run portable -- --target x86_64-pc-windows-msvc`），
压缩用 7z，缺 7z 时回退 `tar -a`。

## 可选：内置 Rust 核心

pi-web 正在把后端逻辑逐步迁到 Rust（见 pi-web 仓库的 `RUST-MIGRATION-PLAN.md`）。
目前是**可选**的：payload 里没有 `native/pi-core.node` 时，
`lib/rust-core.ts` 返回 null，一切回退 TypeScript，功能完全正常。
`PI_CORE_MODE` 控制它是否参与出答案：

| 值 | 行为 |
|---|---|
| `off` | Rust 不参与（回退 TS） |
| `shadow` | **默认**。Rust 跑并与 TS 比对，但答案仍由 TS 给出 |
| `on` | Rust 可以出答案；出错即回退 TS |

要让便携版带上它，在 pi-web 源码目录执行一次
`node scripts/build-native.mjs`（需要 Rust 工具链），再跑 `prepare-desktop.mjs`
——它会把 `native/` 拷进 `app/`。缺失时脚本只打印警告，
因为运行时回退是无害的；但**发布流水线**会用 `PI_REQUIRE_RUST_CORE=1`
把它变成硬失败，避免悄悄发布一个其实没走 Rust 的包。

查看实际是否加载：`data\logs\desktop.log` 里的 `[rust-core]` 行，
或在 `PI_CORE_MODE=shadow` 下观察是否出现 `walk diverged` 告警。

> **Cargo workspace 注意**：pi-web 仓库根目录是 Cargo workspace，
> 而本仓库嵌套在它下面。pi-web 的 `Cargo.toml` 因此需要
> `exclude = ["desktop"]`，否则 `tauri build` 会报
> `current package believes it's in a workspace when it's not`。

## 开发者模式

| 能力 | 用法 |
|---|---|
| 启动日志 | **始终写入** `<app data>/logs/desktop.log`（含 app 目录、入口路径、端口、sidecar 全部输出、就绪/失败） |
| 启动失败报告 | 失败时写 `logs/startup-failure.txt` 并自动用记事本打开 + 打开日志目录（不再静默退出） |
| 日志目录 | 托盘右键 → 「打开日志目录」 |
| 开发者工具 | 托盘右键 → 「开发者工具」（已启用 `devtools` 特性，release 版同样可用）；或启动时设 `PI_WEB_DESKTOP_DEBUG=1` 自动打开 |
| 直连本地服务 | 设 `PI_WEB_DESKTOP_DEV_URL=http://127.0.0.1:30141`：**完全跳过 sidecar**，窗口直接打开该地址，窗口标题带 ` (dev)`。前端迭代无需重新组装载荷、无需重装 |
| 环境变量 | `PI_WEB_DESKTOP_DEBUG=1`（自动开 devtools）、`PI_WEB_DESKTOP_DEV_URL=<url>`（跳过 sidecar）、`PI_WEB_DESKTOP=1`（传给 sidecar，供前端识别桌面端） |
| 单实例 | 已注册 `tauri-plugin-single-instance`：再次启动只会把已有窗口拉到前台，不会开第二个实例（两个实例会共用同一个 agent 目录，会写坏会话状态） |
| 退出清理 | 托盘「退出」与 `RunEvent::Exit` 都用 `taskkill /F /T /PID` 结束**整个 sidecar 进程树**；只 `kill()` 直接子进程会留下 Next.js worker 占住端口 |

日志目录实际路径：`%APPDATA%\<identifier>\logs`（Windows，identifier 见 `tauri.conf.json`；便携版则是解压目录下的 `data\logs`）。

排障顺序：启动无窗口 → 打开日志目录 → 看 `desktop.log` 末尾与 `startup-failure.txt` → 常见原因是 sidecar 参数或 `app/` 载荷缺失。

## 模型 API 故障排查

模型配置不在壳里，而在 pi-web 的 agent 目录：

| 形态 | 路径 |
|---|---|
| 便携版 | `<解压目录>\data\pi\agent\models.json` |
| 安装版 | `%USERPROFILE%\.pi\agent\models.json` |

改完**无需重启**：pi-web 会在下一次请求时重新读取。

### `Error: Unexpected message role.`

模型本身报的错，不是壳的问题。pi-ai 对 `reasoning: true` 的模型默认把系统指令
以 `role: "developer"` 发送（OpenAI o-series 的约定）：

```js
// node_modules/@earendil-works/pi-ai/dist/api/openai-completions.js:899
const instructionRole = model.reasoning && compat.supportsDeveloperRole ? "developer" : "system";
```

OpenRouter / SGLang 风格的网关不认识 `developer`，直接 400。给对应模型关掉即可：

```json
{
  "id": "Qwen3.8-27B",
  "reasoning": true,
  "compat": { "supportsDeveloperRole": false }
}
```

模型级 `compat` 会覆盖自动探测（`openai-completions.js:1316`），所以这一处就够。
判断方法：用同一个 key 直接打端点，`role: "system"` 返回 200 而 `role: "developer"`
返回 `Unexpected message role.` 即为此问题。

### `Error: Connection error.`

`baseUrl` 指向的服务没起来或不可达。先确认端口在监听：

```bash
netstat -ano -p TCP | findstr ":8808"
```

注意 Node 的 `fetch`（undici）**不读** `HTTP_PROXY` / `HTTPS_PROXY`，所以"curl 能通"
不代表 pi-web 能通；反过来，本机开了代理也不代表壳里的 sidecar 会走它。

### 用 DEV_URL 迭代前端

```bash
# 终端 1：单独跑 pi-web 的 dev/prod 服务（自己控制源码与热重载）
cd src-app && PI_WEB_DESKTOP=1 node bin/pi-web.js -H 127.0.0.1 -p 30141 --no-open

# 终端 2：壳直接连上去，不启 sidecar
PI_WEB_DESKTOP_DEV_URL=http://127.0.0.1:30141 PI_WEB_DESKTOP_DEBUG=1 npm run dev
```

`PI_WEB_DESKTOP=1` 要由你自己传给服务进程（sidecar 模式下才是壳注入的），前端据此隐藏浏览器专有 UI。

## 发布（GitHub Actions）

一次性准备：

```bash
npx tauri signer generate -w ~/.tauri/pi-web.key
# 1) 把输出的 pubkey 填进 src-tauri/tauri.conf.json > plugins.updater.pubkey
# 2) 私钥离线备份（丢失后已安装用户无法再升级）
# 3) 仓库 Settings → Secrets → Actions 添加：
#      TAURI_SIGNING_PRIVATE_KEY
#      TAURI_SIGNING_PRIVATE_KEY_PASSWORD（若设置了密码）
```

日常发布：

```bash
# 三处 version 保持一致：package.json / src-tauri/Cargo.toml / src-tauri/tauri.conf.json
git tag v0.12.0 && git push origin v0.12.0
```

工作流会：安装依赖 → `next build` → `npm prune --omit=dev` →
组装载荷（下载官方 Node 并校验 SHA256，node-pty 用自带 prebuild，**不要** `--build-from-source`）→
`tauri build` → 签名 → 发布 **草稿 Release**
（含 `*.exe`、`*.sig`、`latest.json`）。人工验收后再手动发布草稿。

Windows runner 自带 MSVC Build Tools 与 NSIS，因此不需要本地工具链。

## 待办（首次出包前）

- [ ] 替换 `app-icon.png` 为真实品牌图标（≥1024×1024 PNG）。
- [ ] 填写 `tauri.conf.json > plugins.updater.pubkey`。
- [ ] Pi Web 侧：读取 `PI_WEB_DESKTOP=1` 隐藏浏览器专有 UI（PWA 安装提示、LAN 提示、密码设置入口）。
- [ ] Pi Web 侧：自定义标题栏（需 `data-tauri-drag-region`，否则无边框窗口无法拖动）。
