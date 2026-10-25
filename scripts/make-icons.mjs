// Generates the DroidWin icon set. No image library: a raw RGBA PNG encoder plus
// a single-entry ICO wrapper is all a square lacquer mark needs.
//
//   node scripts/make-icons.mjs
//
// Palette: void canvas #05070A, vermilion lacquer #E0231C, sage rule #BBCBBD.

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const OUT = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "icons");
mkdirSync(OUT, { recursive: true });

const CANVAS = [0x05, 0x07, 0x0a];
const LACQUER = [0xe0, 0x23, 0x1c];
const SAGE = [0xbb, 0xcb, 0xbd];

function pixels(size) {
  const px = Buffer.alloc(size * size * 4);
  const inset = Math.round(size * 0.16);
  const markTop = Math.round(size * 0.30);
  const markBottom = Math.round(size * 0.70);
  const ruleAt = Math.round(size * 0.78);
  const ruleThickness = Math.max(1, Math.round(size * 0.02));

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = (y * size + x) * 4;
      let colour = CANVAS;
      // Vermilion block: a lacquered doorway, inset from the canvas.
      if (x >= inset && x < size - inset && y >= inset && y < size - inset) colour = LACQUER;
      // Sage rule: the shoji divider under the block.
      if (x >= inset && x < size - inset && y >= ruleAt && y < ruleAt + ruleThickness) colour = SAGE;
      // Negative space: the gap between the block and the rule reads as a sill.
      if (y >= markBottom && y < ruleAt) colour = CANVAS;
      if (y >= inset && y < markTop && x >= inset && x < size - inset) colour = LACQUER;
      px[i] = colour[0];
      px[i + 1] = colour[1];
      px[i + 2] = colour[2];
      px[i + 3] = 255;
    }
  }
  return px;
}

function crc32(buf) {
  let table = crc32.table;
  if (!table) {
    table = crc32.table = new Int32Array(256);
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      table[n] = c;
    }
  }
  let crc = -1;
  for (const b of buf) crc = (crc >>> 8) ^ table[(crc ^ b) & 0xff];
  return (crc ^ -1) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

function png(size) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const raw = pixels(size);
  const stride = size * 4;
  const filtered = Buffer.alloc((stride + 1) * size);
  for (let y = 0; y < size; y++) {
    filtered[y * (stride + 1)] = 0; // filter: none
    raw.copy(filtered, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(filtered, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function ico(images) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  const entries = [];
  let offset = 6 + images.length * 16;
  for (const { size, data } of images) {
    const entry = Buffer.alloc(16);
    entry[0] = size >= 256 ? 0 : size;
    entry[1] = size >= 256 ? 0 : size;
    entry[2] = 0;
    entry[3] = 0;
    entry.writeUInt16LE(1, 4);
    entry.writeUInt16LE(32, 6);
    entry.writeUInt32LE(data.length, 8);
    entry.writeUInt32LE(offset, 12);
    entries.push(entry);
    offset += data.length;
  }
  return Buffer.concat([header, ...entries, ...images.map((i) => i.data)]);
}

for (const size of [32, 128, 256]) {
  writeFileSync(join(OUT, `${size}x${size}.png`), png(size));
}
writeFileSync(
  join(OUT, "icon.ico"),
  ico([32, 128, 256].map((size) => ({ size, data: png(size) }))),
);
console.log("wrote icons to", OUT);
