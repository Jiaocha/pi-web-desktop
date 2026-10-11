#!/usr/bin/env node
/**
 * Assembles the runtime payload the Tauri shell ships:
 *
 *   1. Downloads the official Node executable for the target triple and drops it
 *      in src-tauri/binaries/node-<target>[.exe] (Tauri externalBin naming).
 *   2. Copies the built Next.js app (.next, public, bin, package.json) and the
 *      production node_modules into ./app, which tauri.conf.json maps to
 *      $RESOURCE/app.
 *   3. Fails loudly when the node-pty native binary is missing, because a
 *      bundle without it installs fine and only breaks when a terminal opens.
 *
 * Usage:
 *   node scripts/prepare-desktop.mjs --target x86_64-pc-windows-msvc
 *   node scripts/prepare-desktop.mjs --target aarch64-apple-darwin --src-app ../src-app
 */
"use strict";

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, "..");

function parseArgs(argv) {
  // src-app is a subdirectory of this project (CI checks Pi Web out into it,
  // locally you clone or submodule it there).
  const args = { target: "", srcApp: "src-app", nodeVersion: "22.22.0" };
  const flags = { "--target": "target", "--src-app": "srcApp", "--node-version": "nodeVersion" };
  for (let i = 0; i < argv.length; i += 1) {
    const key = flags[argv[i]];
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

/** x86_64-pc-windows-msvc -> { os: "win", arch: "x64", ext: ".exe" } */
function targetToNodeDist(target) {
  const [archPart, , osPart] = target.split("-");
  const archMap = { x86_64: "x64", aarch64: "arm64", armv7: "armv7l", i686: "x86" };
  const osMap = { windows: "win", darwin: "darwin", linux: "linux" };
  const arch = archMap[archPart];
  const os = osMap[osPart];
  if (!arch || !os) {
    console.error(`Unsupported target triple: ${target}`);
    process.exit(1);
  }
  return { os, arch, ext: os === "win" ? ".exe" : "" };
}

async function download(url) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`GET ${url} -> HTTP ${response.status}`);
  }
  return Buffer.from(await response.arrayBuffer());
}

async function downloadWithChecksum(url, checksumsUrl, fileName) {
  const [binary, sums] = await Promise.all([download(url), download(checksumsUrl)]);
  const expected = sums
    .toString("utf8")
    .split("\n")
    .map((line) => line.trim().split(/\s+/))
    .find(([, name]) => name === fileName)?.[0];

  if (!expected) {
    throw new Error(`No checksum entry for ${fileName} in ${checksumsUrl}`);
  }
  const actual = createHash("sha256").update(binary).digest("hex");
  if (actual !== expected) {
    throw new Error(`Checksum mismatch for ${fileName}: ${actual} != ${expected}`);
  }
  console.log(`✓ ${fileName} sha256 verified`);
  return binary;
}

async function installNodeRuntime(target, version, binariesDir) {
  const { os, arch, ext } = targetToNodeDist(target);
  const base = `https://nodejs.org/dist/v${version}`;
  const fileName = `node-v${version}-${os}-${arch}`;
  const outFile = path.join(binariesDir, `node-${target}${ext}`);
  fs.mkdirSync(binariesDir, { recursive: true });

  // Windows publishes a bare node.exe alongside the archive; other platforms
  // need the tarball extracted.
  if (os === "win") {
    const binary = await downloadWithChecksum(
      `${base}/win-${arch}/node.exe`,
      `${base}/SHASUMS256.txt`,
      `win-${arch}/node.exe`,
    );
    fs.writeFileSync(outFile, binary);
  } else {
    const archive = `${fileName}.tar.gz`;
    const tarPath = path.join(binariesDir, archive);
    const buffer = await downloadWithChecksum(`${base}/${archive}`, `${base}/SHASUMS256.txt`, archive);
    fs.writeFileSync(tarPath, buffer);
    fs.mkdirSync(path.join(binariesDir, "_extract"), { recursive: true });
    execFileSync("tar", ["-xzf", tarPath, "-C", path.join(binariesDir, "_extract")], {
      stdio: "inherit",
    });
    const extracted = path.join(binariesDir, "_extract", fileName, "bin", "node");
    if (!fs.existsSync(extracted)) {
      throw new Error(`node binary not found in archive at ${extracted}`);
    }
    fs.copyFileSync(extracted, outFile);
    fs.rmSync(path.join(binariesDir, "_extract"), { recursive: true, force: true });
    fs.rmSync(tarPath, { force: true });
  }

  fs.chmodSync(outFile, 0o755);
  console.log(`✓ sidecar runtime: ${path.relative(projectDir, outFile)}`);
}

function copyIfExists(source, destination, filter) {
  if (!fs.existsSync(source)) return false;
  fs.cpSync(source, destination, { recursive: true, filter, dereference: false });
  return true;
}

function assembleApp(srcAppDir, appDir) {
  fs.rmSync(appDir, { recursive: true, force: true });
  fs.mkdirSync(appDir, { recursive: true });

  // .next/cache and .next/dev are build scratch space; shipping them inflates
  // the installer and can pin stale dev artifacts.
  const nextDir = path.join(srcAppDir, ".next");
  const scratch = new Set([path.join(nextDir, "cache"), path.join(nextDir, "dev")]);
  const skipScratch = (source) => !scratch.has(source);

  const copied = [
    copyIfExists(nextDir, path.join(appDir, ".next"), skipScratch),
    copyIfExists(path.join(srcAppDir, "public"), path.join(appDir, "public")),
    // bin/pi-web.js is the official launcher: it rotates preview secrets and
    // keeps the run records the CLI writes.
    copyIfExists(path.join(srcAppDir, "bin"), path.join(appDir, "bin")),
  ];

  const pkg = path.join(srcAppDir, "package.json");
  if (!fs.existsSync(pkg)) throw new Error(`No package.json in ${srcAppDir}`);
  fs.copyFileSync(pkg, path.join(appDir, "package.json"));
  if (!copied[0]) throw new Error("No .next build output; run `npm run build` first.");

  const modules = path.join(srcAppDir, "node_modules");
  if (!fs.existsSync(modules)) throw new Error("No node_modules; run `npm ci` first.");
  fs.cpSync(modules, path.join(appDir, "node_modules"), { recursive: true, dereference: false });
}

/**
 * node-pty 1.2.0-beta.15 ships prebuilds for win32-x64 / win32-arm64 /
 * linux-x64 / linux-arm64 (verified on GitHub runners), so the normal
 * `npm ci` install is enough. A source build is only needed when no prebuild
 * matches, and that requires Visual Studio / build-essential to be present.
 */
function assertNativePty(appDir, target) {
  const root = path.join(appDir, "node_modules", "node-pty");
  const [arch, , os] = target.split("-");
  const platformDir = `${os}-${arch}`; // e.g. win32-x64

  const candidates = [
    path.join(root, "prebuilds", platformDir),
    path.join(root, "build", "Release"),
  ];
  const found = candidates.find((candidate) => {
    if (!fs.existsSync(candidate)) return false;
    return fs
      .readdirSync(candidate, { withFileTypes: true, recursive: true })
      .some((entry) => entry.isFile() && /\.node$|OpenConsole\.exe$/.test(entry.name));
  });

  if (!found) {
    throw new Error(
      `No node-pty native binary for ${platformDir}. Expected one of:\n  ${candidates.join("\n  ")}\n` +
        "A source build (`npm rebuild node-pty --build-from-source`) needs Visual Studio (Windows) " +
        "or build-essential + python3 (Debian/Ubuntu).",
    );
  }
  console.log(`✓ node-pty native binary: ${path.relative(appDir, found)}`);
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

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const srcAppDir = path.resolve(projectDir, args.srcApp);
  const appDir = path.join(projectDir, "app");
  const binariesDir = path.join(projectDir, "src-tauri", "binaries");

  console.log(`target=${args.target} node=v${args.nodeVersion} src=${path.relative(projectDir, srcAppDir)}`);

  await installNodeRuntime(args.target, args.nodeVersion, binariesDir);
  assembleApp(srcAppDir, appDir);
  assertNativePty(appDir, args.target);

  const buildId = path.join(appDir, ".next", "BUILD_ID");
  if (!fs.existsSync(buildId)) throw new Error(".next/BUILD_ID missing; the build did not finish.");

  console.log(`✓ app payload ${mb(sizeOf(appDir))} (node_modules ${mb(sizeOf(path.join(appDir, "node_modules")))})`);
}

main().catch((error) => {
  console.error(`\nprepare-desktop failed: ${error.message}`);
  process.exit(1);
});
