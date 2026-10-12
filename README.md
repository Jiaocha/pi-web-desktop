# Pi Web Desktop

把 [Pi Web](https://github.com/agegr/pi-web)（全栈 Next.js 应用）打包成 Windows 桌面应用：
Tauri v2 只做窗口壳与生命周期，真正的服务由内置的 Node sidecar 运行
`bin/pi-web.js start`，WebView2 指向 `http://127.0.0.1:<动态端口>`。

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

## 开发者模式

| 能力 | 用法 |
|---|---|
| 启动日志 | **始终写入** `<app data>/logs/desktop.log`（含 app 目录、入口路径、端口、sidecar 全部输出、就绪/失败） |
| 启动失败报告 | 失败时写 `logs/startup-failure.txt` 并自动用记事本打开 + 打开日志目录（不再静默退出） |
| 日志目录 | 托盘右键 → 「打开日志目录」 |
| 开发者工具 | 托盘右键 → 「开发者工具」（已启用 `devtools` 特性，release 版同样可用）；或启动时设 `PI_WEB_DESKTOP_DEBUG=1` 自动打开 |
| 环境变量 | `PI_WEB_DESKTOP_DEBUG=1`（自动开 devtools）、`PI_WEB_DESKTOP=1`（传给 sidecar，供前端识别桌面端） |

日志目录实际路径：`%LOCALAPPDATA%\<identifier>\logs`（Windows，identifier 见 `tauri.conf.json`）。

排障顺序：启动无窗口 → 打开日志目录 → 看 `desktop.log` 末尾与 `startup-failure.txt` → 常见原因是 sidecar 参数或 `app/` 载荷缺失。

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

工作流会：安装依赖 → node-pty 源码构建 → `next build --webpack` → `npm prune --omit=dev` →
组装载荷（下载官方 Node 并校验 SHA256）→ `tauri build` → 签名 → 发布 **草稿 Release**
（含 `*.exe`、`*.sig`、`latest.json`）。人工验收后再手动发布草稿。

Windows runner 自带 MSVC Build Tools 与 NSIS，因此不需要本地工具链。

## 待办（首次出包前）

- [ ] 替换 `app-icon.png` 为真实品牌图标（≥1024×1024 PNG）。
- [ ] 填写 `tauri.conf.json > plugins.updater.pubkey`。
- [ ] Pi Web 侧：读取 `PI_WEB_DESKTOP=1` 隐藏浏览器专有 UI（PWA 安装提示、LAN 提示、密码设置入口）。
- [ ] Pi Web 侧：自定义标题栏（需 `data-tauri-drag-region`，否则无边框窗口无法拖动）。
