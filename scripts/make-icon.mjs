#!/usr/bin/env node
/**
 * Writes a placeholder app icon so `tauri icon` has a source image without
 * committing a binary to the repository. Replace it with a real brand asset:
 *
 *   node scripts/make-icon.mjs app-icon.png   # then commit app-icon.png
 *   npx tauri icon app-icon.png -o src-tauri/icons
 *
 * The generated image is a 1024x1024 RGBA PNG (solid rounded square + ring),
 * which is the minimum `tauri icon` needs to derive every platform icon.
 */
"use strict";

import { deflateSync } from "node:zlib";
import fs from "node:fs";
import path from "node:path";

const SIZE = 1024;

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(buffer) {
  let crc = -1;
  for (const byte of buffer) crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ -1) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body), 0);
  return Buffer.concat([length, body, crc]);
}

/** Draws a rounded square with a concentric ring, fully opaque inside. */
function pixel(x, y) {
  const size = SIZE;
  const pad = size * 0.06;
  const inner = size - pad * 2;
  const radius = size * 0.22;

  // Rounded-rectangle distance test.
  const dx = Math.max(pad - x, 0, x - (size - pad));
  const dy = Math.max(pad - y, 0, y - (size - pad));
  const corner = Math.hypot(Math.max(dx - radius, 0), Math.max(dy - radius, 0));
  if (Math.hypot(dx, dy) > radius + 0.5 && corner > 0.5) return [0, 0, 0, 0];

  const cx = x - size / 2;
  const cy = y - size / 2;
  const r = Math.hypot(cx, cy) / (size / 2);
  if (r > 0.52 && r < 0.66) return [255, 255, 255, 255]; // ring
  if (r <= 0.52) return [255, 255, 255, 255]; // glyph block
  return [37, 99, 235, 255]; // #2563EB
}

function encodePng() {
  const stride = SIZE * 4 + 1;
  const raw = Buffer.alloc(stride * SIZE);
  for (let y = 0; y < SIZE; y += 1) {
    const row = y * stride;
    raw[row] = 0; // filter: none
    for (let x = 0; x < SIZE; x += 1) {
      const offset = row + 1 + x * 4;
      const [r, g, b, a] = pixel(x, y);
      raw[offset] = r;
      raw[offset + 1] = g;
      raw[offset + 2] = b;
      raw[offset + 3] = a;
    }
  }

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(SIZE, 0);
  ihdr.writeUInt32BE(SIZE, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // colour type: RGBA
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const output = path.resolve(process.argv[2] ?? "app-icon.png");
fs.mkdirSync(path.dirname(output), { recursive: true });
fs.writeFileSync(output, encodePng());
console.log(`✓ wrote ${output} (${SIZE}x${SIZE} PNG)`);
