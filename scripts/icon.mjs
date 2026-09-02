// 의존성 없이 앱 아이콘 원본 PNG(1024x1024)를 그린다.
// 이 파일을 `npx tauri icon`에 넘기면 플랫폼별 아이콘이 생성된다.
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const S = 1024;
const buf = new Uint8Array(S * S * 4);

function px(x, y, [r, g, b], a = 1) {
  if (x < 0 || y < 0 || x >= S || y >= S || a <= 0) return;
  const i = (y * S + x) * 4;
  const src = [r, g, b];
  for (let c = 0; c < 3; c++) {
    buf[i + c] = Math.round(buf[i + c] * (1 - a) + src[c] * a);
  }
  buf[i + 3] = Math.round(buf[i + 3] * (1 - a) + 255 * a);
}

// 경계에서 부드럽게 떨어지는 커버리지 값.
function coverage(d) {
  return Math.max(0, Math.min(1, 0.5 - d));
}

function roundRect(x0, y0, w, h, r, color) {
  for (let y = Math.floor(y0 - 2); y < y0 + h + 2; y++) {
    for (let x = Math.floor(x0 - 2); x < x0 + w + 2; x++) {
      // 라운드 사각형까지의 부호 있는 거리
      const dx = Math.max(x0 + r - x, 0, x - (x0 + w - r));
      const dy = Math.max(y0 + r - y, 0, y - (y0 + h - r));
      const d = Math.hypot(dx, dy) - r;
      px(x, y, color, coverage(d));
    }
  }
}

function ellipse(cx, cy, rx, ry, color) {
  for (let y = Math.floor(cy - ry - 2); y <= cy + ry + 2; y++) {
    for (let x = Math.floor(cx - rx - 2); x <= cx + rx + 2; x++) {
      const nx = (x - cx) / rx;
      const ny = (y - cy) / ry;
      const d = (Math.hypot(nx, ny) - 1) * Math.min(rx, ry);
      px(x, y, color, coverage(d));
    }
  }
}

// 두께가 있는 선분.
function line(x1, y1, x2, y2, w, color) {
  const vx = x2 - x1;
  const vy = y2 - y1;
  const len2 = vx * vx + vy * vy || 1;
  const minX = Math.min(x1, x2) - w;
  const maxX = Math.max(x1, x2) + w;
  const minY = Math.min(y1, y2) - w;
  const maxY = Math.max(y1, y2) + w;
  for (let y = Math.floor(minY); y <= maxY; y++) {
    for (let x = Math.floor(minX); x <= maxX; x++) {
      let t = ((x - x1) * vx + (y - y1) * vy) / len2;
      t = Math.max(0, Math.min(1, t));
      const d = Math.hypot(x - (x1 + t * vx), y - (y1 + t * vy)) - w / 2;
      px(x, y, color, coverage(d));
    }
  }
}

const BG = [16, 20, 28];
const SHELL = [61, 70, 87];
const SCREEN = [13, 18, 25];
const TEAL = [94, 234, 212];
const RED = [244, 104, 95];

// 배경
roundRect(0, 0, S, S, 220, BG);

// 안테나
line(512, 236, 512, 168, 26, [74, 84, 104]);
ellipse(512, 152, 34, 34, RED);

// 머리
roundRect(196, 236, 632, 520, 118, SHELL);
roundRect(268, 308, 488, 340, 78, SCREEN);

// 귀
roundRect(126, 400, 60, 150, 28, [57, 66, 84]);
roundRect(838, 400, 60, 150, 28, [57, 66, 84]);

// 찌그러진 눈
line(360, 430, 452, 500, 34, TEAL);
line(452, 430, 360, 500, 34, TEAL);
line(572, 430, 664, 500, 34, TEAL);
line(664, 430, 572, 500, 34, TEAL);

// 찌푸린 입
line(400, 588, 512, 548, 30, TEAL);
line(512, 548, 624, 588, 30, TEAL);

// 충격 균열
line(300, 300, 372, 396, 22, [8, 11, 17]);
line(372, 396, 316, 448, 22, [8, 11, 17]);
line(316, 448, 396, 540, 22, [8, 11, 17]);

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crcBuf = Buffer.alloc(4);
  crcBuf.writeUInt32BE(crc32(body) >>> 0);
  return Buffer.concat([len, body, crcBuf]);
}

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(bytes) {
  let c = 0xffffffff;
  for (const b of bytes) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return c ^ 0xffffffff;
}

const raw = Buffer.alloc(S * (S * 4 + 1));
for (let y = 0; y < S; y++) {
  raw[y * (S * 4 + 1)] = 0; // 필터 타입 None
  Buffer.from(buf.buffer, y * S * 4, S * 4).copy(raw, y * (S * 4 + 1) + 1);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(S, 0);
ihdr.writeUInt32BE(S, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // RGBA
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const out = join(root, "src-tauri", "icons", "source.png");
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, png);
console.log(`아이콘 원본 생성: ${out}`);
