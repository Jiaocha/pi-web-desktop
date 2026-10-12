#!/usr/bin/env node
/**
 * Assembles the portable ("green") Windows build: a folder that runs straight
 * out of a zip, with no installer, no registry writes and no per-user data
 * (a portable.txt marker next to the exe switches the shell to <root>/data).
 *
 * It reuses the release binary the bundler just produced, so the portable zip
 * costs no extra Rust compile time.
 *
 * Layout it produces:
 *
 *   Pi-Web-Desktop-<version>-portable-win64/
 *     Pi Web Desktop.exe        <- double-click entry point
 *     run-debug.bat             <- foreground run, devtools open, prints exit code
 *     node-<target>.exe         <- bundled Node runtime (do not rename)
 *     vcruntime140.dll ...      <- MSVC runtime, for machines without the redist
 *     app/                      <- Pi Web payload (Next.js + node_modules + bin)
 *     data/                     <- everything written at runtime (created here)
 *     runtime/                  <- WebView2 bootstrapper for machines that lack it
 *     portable.txt              <- marker: switches on portable mode
 *     README.txt                <- short usage note
 *
 * Usage:
 *   node scripts/make-portable.mjs --target x86_64-pc-windows-msvc
 */
"use strict";

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, "..");

/** UTF-8 with BOM: Notepad renders Chinese correctly on every Windows version. */
function writeText(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, "\uFEFF" + text.replace(/\n/g, "\r\n"), "utf8");
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function archLabel(target) {
  const [arch, , os] = target.split("-");
  const osLabel = { windows: "win", darwin: "mac", linux: "linux" }[os] || os;
  const archMap = { x86_64: "64", aarch64: "arm64", i686: "32" };
  return `${osLabel}${archMap[arch] || arch}`;
}

/**
 * MSVC runtime DLLs. The Rust executable links the Universal CRT (present on
 * every Windows 10+), and the official Node build is self-contained too, but
 * shipping these costs well under a megabyte and removes the last reason a
 * clean machine could fail to start.
 */
function copyVcRuntime(rootDir) {
  const systemRoot = process.env.SystemRoot || "C:\\Windows";
  const from = path.join(systemRoot, "System32");
  const dlls = [
    "vcruntime140.dll",
    "vcruntime140_1.dll",
    "msvcp140.dll",
    "concrt140.dll",
    "vcomp140.dll",
  ];
  const copied = [];
  for (const dll of dlls) {
    const source = path.join(from, dll);
    if (!fs.existsSync(source)) continue;
    fs.copyFileSync(source, path.join(rootDir, dll));
    copied.push(dll);
  }
  console.log(`✓ MSVC runtime: ${copied.length ? copied.join(", ") : "none found on this machine"}`);
  return copied;
}

/**
 * Best effort: the bootstrapper is only needed on machines without WebView2
 * (Windows 10 21H2+ and Windows 11 ship it). A blocked network must not fail
 * the build, so every failure downgrades to a warning.
 */
async function fetchWebView2Bootstrapper(runtimeDir) {
  const url = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
  const target = path.join(runtimeDir, "MicrosoftEdgeWebview2Setup.exe");
  try {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    if (bytes.length < 50_000 || bytes.subarray(0, 2).toString("latin1") !== "MZ") {
      throw new Error(`unexpected payload (${bytes.length} bytes)`);
    }
    fs.mkdirSync(runtimeDir, { recursive: true });
    fs.writeFileSync(target, bytes);
    console.log(`✓ WebView2 bootstrapper: ${(bytes.length / 1024).toFixed(0)} KB`);
    return true;
  } catch (error) {
    console.log(`! WebView2 bootstrapper unavailable (${error.message}); shipping the batch file only`);
    return false;
  }
}

function zipFolder(distDir, folderName, zipFile) {
  const sevenZip = [
    "7z",
    "C:\\Program Files\\7-Zip\\7z.exe",
    "C:\\Program Files (x86)\\7-Zip\\7z.exe",
  ];
  for (const binary of sevenZip) {
    try {
      execFileSync(binary, ["a", "-tzip", "-mx=5", "-bso0", zipFile, folderName], {
        cwd: distDir,
        stdio: "inherit",
      });
      return binary;
    } catch {
      // try the next candidate
    }
  }
  execFileSync("tar", ["-a", "-c", "-f", zipFile, folderName], {
    cwd: distDir,
    stdio: "inherit",
  });
  return "tar";
}

function sizeOf(dir) {
  if (!fs.existsSync(dir)) return 0;
  let total = 0;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    total += entry.isDirectory() ? sizeOf(full) : fs.statSync(full).size;
  }
  return total;
}

const mb = (bytes) => `${(bytes / 1024 / 1024).toFixed(1)} MB`;

function parseArgs(argv) {
  const args = { target: "", version: "" };
  for (let i = 0; i < argv.length; i += 1) {
    const key = { "--target": "target", "--version": "version" }[argv[i]];
    if (key && argv[i + 1]) {
      args[key] = argv[i + 1];
      i += 1;
    }
  }
  if (!args.target) {
    console.error("Missing --target (e.g. x86_64-pc-windows-msvc)");
    process.exit(1);
  }
  return args;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const config = readJson(path.join(projectDir, "src-tauri", "tauri.conf.json"));
  const cargo = fs.readFileSync(path.join(projectDir, "src-tauri", "Cargo.toml"), "utf8");
  const binName = /^name\s*=\s*"([^"]+)"/m.exec(cargo)?.[1];
  if (!binName) throw new Error("Cannot read the crate name from src-tauri/Cargo.toml");

  const version = args.version || config.version;
  const productName = config.productName;
  const ext = args.target.includes("windows") ? ".exe" : "";

  const builtExe = path.join(
    projectDir,
    "src-tauri",
    "target",
    args.target,
    "release",
    `${binName}${ext}`,
  );
  if (!fs.existsSync(builtExe)) {
    throw new Error(`Release binary not found: ${builtExe}\nRun the Tauri build first.`);
  }
  const sidecar = path.join(
    projectDir,
    "src-tauri",
    "binaries",
    `node-${args.target}${ext}`,
  );
  if (!fs.existsSync(sidecar)) {
    throw new Error(`Node sidecar not found: ${sidecar}\nRun scripts/prepare-desktop.mjs first.`);
  }
  const appDir = path.join(projectDir, "app");
  if (!fs.existsSync(path.join(appDir, "bin", "pi-web.js"))) {
    throw new Error(`Payload missing at ${appDir}; run scripts/prepare-desktop.mjs first.`);
  }

  const folderName = `Pi-Web-Desktop-${version}-portable-${archLabel(args.target)}`;
  const distDir = path.join(projectDir, "dist");
  const rootDir = path.join(distDir, folderName);
  fs.rmSync(rootDir, { recursive: true, force: true });
  fs.mkdirSync(rootDir, { recursive: true });

  fs.copyFileSync(builtExe, path.join(rootDir, `${productName}${ext}`));
  fs.copyFileSync(sidecar, path.join(rootDir, `node-${args.target}${ext}`));
  fs.cpSync(appDir, path.join(rootDir, "app"), { recursive: true, dereference: false });
  console.log(`✓ exe + sidecar + payload (${mb(sizeOf(path.join(rootDir, "app")))} app/)`);

  copyVcRuntime(rootDir);

  // Marker that switches the shell to portable mode. It doubles as a readme
  // because it is the first thing someone opening the folder will click.
  writeText(
    path.join(rootDir, "portable.txt"),
    [
      "Pi Web Desktop - portable mode marker",
      "",
      "This file tells Pi Web Desktop to be fully portable. Delete it and the",
      "app goes back to storing its data under %APPDATA% - do that only if you",
      "know why you want it.",
      "",
      "With this file present:",
      "  logs          -> data\\logs\\desktop.log",
      "  browser data  -> data\\webview",
      "  sessions etc. -> data\\pi\\agent",
      "",
      "Nothing is written to the registry, %APPDATA% or Program Files.",
    ].join("\n"),
  );

  writeText(
    path.join(rootDir, "data", "README.txt"),
    [
      "Everything Pi Web Desktop writes at runtime lands in this folder, using",
      "paths relative to the folder you extracted. Copy or move the whole",
      "Pi-Web-Desktop folder (this one included) and your sessions come with it.",
      "",
      "  logs\\     desktop.log, startup-failure.txt",
      "  webview\\  browser profile (cookies, localStorage)",
      "  pi\\agent\\ sessions, settings, server run records",
      "",
      "Deleting this folder resets the app to a clean state.",
    ].join("\n"),
  );

  // ASCII on purpose: batch files with non-ASCII bytes render as mojibake
  // depending on the console code page.
  fs.writeFileSync(
    path.join(rootDir, "run-debug.bat"),
    [
      "@echo off",
      "cd /d \"%~dp0\"",
      "set PI_WEB_DESKTOP_DEBUG=1",
      "echo Starting Pi Web Desktop (debug mode, devtools open) ...",
      "echo Logs: %~dp0data\\logs\\desktop.log",
      "echo.",
      `"%~dp0${productName}${ext}"`,
      "echo.",
      "echo Exit code: %ERRORLEVEL%",
      "echo If it failed, open data\\logs\\desktop.log or startup-failure.txt.",
      "pause",
      "",
    ].join("\r\n"),
    "utf8",
  );

  const runtimeDir = path.join(rootDir, "runtime");
  const hasBootstrapper = await fetchWebView2Bootstrapper(runtimeDir);
  fs.writeFileSync(
    path.join(runtimeDir, "install-webview2.bat"),
    [
      "@echo off",
      "cd /d \"%~dp0\"",
      "echo This installs the Edge WebView2 runtime, needed only on older Windows.",
      "echo Windows 10 21H2+ and Windows 11 already have it.",
      "echo.",
      "if not exist \"MicrosoftEdgeWebview2Setup.exe\" (",
      "  echo MicrosoftEdgeWebview2Setup.exe is missing next to this file.",
      "  echo Download it from https://developer.microsoft.com/microsoft-edge/webview2/",
      "  pause",
      "  exit /b 1",
      ")",
      "echo Downloading and installing (needs an internet connection) ...",
      "start /wait \"\" \"MicrosoftEdgeWebview2Setup.exe\" /silent /install",
      "echo Done. Exit code: %ERRORLEVEL%",
      "pause",
      "",
    ].join("\r\n"),
    "utf8",
  );
  if (!hasBootstrapper) {
    writeText(
      path.join(runtimeDir, "README.txt"),
      [
        "install-webview2.bat installs the Edge WebView2 runtime for machines that",
        "do not ship it (Windows 10 21H2+ and Windows 11 do).",
        "",
        "The bootstrapper could not be downloaded when this package was built, so",
        "it is not included. Put MicrosoftEdgeWebview2Setup.exe next to the batch",
        "file before running it:",
        "  https://developer.microsoft.com/microsoft-edge/webview2/",
      ].join("\n"),
    );
  }

  writeText(
    path.join(rootDir, "README.txt"),
    [
      `${productName} ${version} - portable edition`,
      "",
      "HOW TO START",
      `  1. Double-click "${productName}${ext}".`,
      "  2. That is all. Nothing is installed.",
      "",
      "  If nothing happens, run run-debug.bat instead: it opens devtools, keeps",
      "  the console open and prints the exit code. The log it points at is",
      "  data\\logs\\desktop.log, and a failed start also writes",
      "  data\\logs\\startup-failure.txt.",
      "",
      "WHAT IS IN THIS FOLDER",
      `  ${productName}${ext}   the application`,
      `  node-${args.target}${ext}`,
      "                       the bundled Node.js runtime; the app looks it up",
      "                       by this exact name, so do not rename or delete it",
      "  app\\                 Pi Web itself (server code, web assets, modules)",
      "  data\\                everything written at runtime: logs, browser",
      "                       profile, sessions and settings",
      "  runtime\\             optional Edge WebView2 installer for old Windows",
      "  vcruntime140*.dll    Microsoft C runtime, for machines without the",
      "                       Visual C++ redistributable",
      "  portable.txt         keeps the app portable; see the file itself",
      "  run-debug.bat        start with devtools and a visible console",
      "",
      "PORTABILITY",
      "  The whole folder is self-contained. Put it on a USB stick and run it",
      "  elsewhere; your sessions travel in data\\. No administrator rights, no",
      "  installer, nothing written to the registry, Program Files or %APPDATA%.",
      "",
      "  One caveat: Windows SmartScreen warns about unsigned executables. The",
      "  build is not code-signed yet, so click More info -> Run anyway.",
      "",
      "REQUIREMENTS",
      "  - Windows 10 21H2 / Windows 11 (x64), or older Windows with the",
      "    WebView2 runtime installed - see runtime\\install-webview2.bat",
      "  - About 1.5 GB of free space uncompressed",
      "",
      "UPDATING",
      "  A portable copy cannot self-update (that would run an installer).",
      "  Download the newer zip and copy your data\\ folder over.",
      "",
      "STOPPING",
      "  Closing the window hides it to the system tray. Use the tray icon's",
      "  Quit menu to exit; that also stops the bundled Node server.",
    ].join("\n"),
  );

  const zipFile = path.join(distDir, `${folderName}.zip`);
  fs.rmSync(zipFile, { force: true });
  const tool = zipFolder(distDir, folderName, zipFile);

  console.log(`✓ portable folder ${mb(sizeOf(rootDir))} -> ${path.relative(projectDir, zipFile)} (${mb(sizeOf(zipFile))}, ${tool})`);
  console.log(`::set-output name=zip::${zipFile}`);
  console.log(`ZIP=${zipFile}`);
}

main().catch((error) => {
  console.error(`\nmake-portable failed: ${error.message}`);
  process.exit(1);
});
