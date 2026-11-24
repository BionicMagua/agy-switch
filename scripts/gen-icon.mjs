// 生成 src-tauri/icons/icon.ico（32x32 PNG 内嵌 ICO）与 icon.png，无第三方依赖。
import { deflateSync } from 'node:zlib';
import { writeFileSync, mkdirSync } from 'node:fs';

const N = 64;
const raw = Buffer.alloc((N * 4 + 1) * N);
for (let y = 0; y < N; y++) {
  raw[y * (N * 4 + 1)] = 0;
  for (let x = 0; x < N; x++) {
    const dx = x - N / 2 + 0.5, dy = y - N / 2 + 0.5;
    const d = Math.hypot(dx, dy);
    const o = y * (N * 4 + 1) + 1 + x * 4;
    if (d < N / 2 - 1) {
      const t = (x + y) / (2 * N);
      raw[o] = 91 + t * 60; raw[o + 1] = 140 - t * 40; raw[o + 2] = 255; raw[o + 3] = 255;
      // 简单的 "⇄" 条纹
      if (Math.abs(dy) < 3 && Math.abs(dx) < 18) { raw[o] = raw[o + 1] = raw[o + 2] = 255; }
    }
  }
}
const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0;
});
const crc = (b) => { let c = 0xffffffff; for (const x of b) c = crcTable[(c ^ x) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
const chunk = (type, data) => {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const c = Buffer.alloc(4); c.writeUInt32BE(crc(td));
  return Buffer.concat([len, td, c]);
};
const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(N, 0); ihdr.writeUInt32BE(N, 4); ihdr[8] = 8; ihdr[9] = 6;
const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0)),
]);
const dir = new URL('../src-tauri/icons/', import.meta.url);
mkdirSync(dir, { recursive: true });
writeFileSync(new URL('icon.png', dir), png);
const hdr = Buffer.alloc(22);
hdr.writeUInt16LE(1, 2); hdr.writeUInt16LE(1, 4);
hdr[6] = N; hdr[7] = N; hdr.writeUInt16LE(1, 10); hdr.writeUInt16LE(32, 12);
hdr.writeUInt32LE(png.length, 14); hdr.writeUInt32LE(22, 18);
writeFileSync(new URL('icon.ico', dir), Buffer.concat([hdr, png]));
console.log('icons generated');
