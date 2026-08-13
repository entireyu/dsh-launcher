// 生成 dsh-launcher 主图标（1024x1024 PNG），输出到项目根目录 icon-source.png
// 设计：蓝色渐变圆角底 + 白色圆环 + 白色播放三角（寓意“一键启动”）
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";

const S = 1024;

// ---------- PNG 编码 ----------
const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([len, typeBuf, data, crc]);
}

// ---------- 向量 / SDF 工具 ----------
const sub = (a, b) => [a[0] - b[0], a[1] - b[1]];
const scale = (a, s) => [a[0] * s, a[1] * s];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1];
const cross = (a, b) => a[0] * b[1] - a[1] * b[0];
const clamp = (v, a, b) => (v < a ? a : v > b ? b : v);
const sign = (v) => (v > 0 ? 1 : v < 0 ? -1 : 0);

function sdRoundRect(px, py, cx, cy, hw, hh, r) {
  const qx = Math.abs(px - cx) - (hw - r);
  const qy = Math.abs(py - cy) - (hh - r);
  const ox = Math.max(qx, 0);
  const oy = Math.max(qy, 0);
  return Math.hypot(ox, oy) + Math.min(Math.max(qx, qy), 0) - r;
}
function sdRing(px, py, cx, cy, midR, halfT) {
  return Math.abs(Math.hypot(px - cx, py - cy) - midR) - halfT;
}
function sdTriangle(p, p0, p1, p2) {
  const e0 = sub(p1, p0), e1 = sub(p2, p1), e2 = sub(p0, p2);
  const v0 = sub(p, p0), v1 = sub(p, p1), v2 = sub(p, p2);
  const pq0 = sub(v0, scale(e0, clamp(dot(v0, e0) / dot(e0, e0), 0, 1)));
  const pq1 = sub(v1, scale(e1, clamp(dot(v1, e1) / dot(e1, e1), 0, 1)));
  const pq2 = sub(v2, scale(e2, clamp(dot(v2, e2) / dot(e2, e2), 0, 1)));
  const s = sign(cross(e0, e2));
  const d = Math.min(Math.min(dot(pq0, pq0), dot(pq1, pq1)), dot(pq2, pq2));
  const inside = Math.min(Math.min(cross(v0, e0), cross(v1, e1)), cross(v2, e2));
  return -s * Math.sqrt(d) * (inside < 0 ? -1 : 1);
}
const cov = (d) => clamp(0.5 - d, 0, 1);

// ---------- 调色板与几何 ----------
const TOP = [62, 107, 232]; // #3E6BE8
const BOTTOM = [20, 38, 92]; // #14265C
const WHITE = [255, 255, 255];

const cx = 512, cy = 512;
const ringMid = 264, ringHalf = 36; // 外径 300 / 内径 228
const triA = [470, 400], triB = [470, 624], triC = [640, 512];

// ---------- 逐像素绘制 ----------
const raw = Buffer.alloc(S * (1 + S * 4));
let o = 0;
for (let y = 0; y < S; y++) {
  raw[o++] = 0; // filter byte
  const t = y / (S - 1);
  for (let x = 0; x < S; x++) {
    // 背景（圆角矩形）
    const dBg = sdRoundRect(x + 0.5, y + 0.5, cx, cy, 512, 512, 224);
    const alpha = cov(dBg);
    let r = 0, g = 0, b = 0;
    if (alpha > 0) {
      // 渐变 + 中心轻微提亮
      let cr = TOP[0] + (BOTTOM[0] - TOP[0]) * t;
      let cg = TOP[1] + (BOTTOM[1] - TOP[1]) * t;
      let cb = TOP[2] + (BOTTOM[2] - TOP[2]) * t;
      const glow = Math.max(0, 1 - Math.hypot(x - cx, y - cy) / 560);
      cr += (255 - cr) * glow * 0.06;
      cg += (255 - cg) * glow * 0.06;
      cb += (255 - cb) * glow * 0.06;
      // 圆环
      const ringA = cov(sdRing(x + 0.5, y + 0.5, cx, cy, ringMid, ringHalf));
      cr += (WHITE[0] - cr) * ringA;
      cg += (WHITE[1] - cg) * ringA;
      cb += (WHITE[2] - cb) * ringA;
      // 播放三角
      const triA_ = cov(sdTriangle([x + 0.5, y + 0.5], triA, triB, triC));
      cr += (WHITE[0] - cr) * triA_;
      cg += (WHITE[1] - cg) * triA_;
      cb += (WHITE[2] - cb) * triA_;
      r = Math.round(cr);
      g = Math.round(cg);
      b = Math.round(cb);
    }
    raw[o++] = r;
    raw[o++] = g;
    raw[o++] = b;
    raw[o++] = Math.round(alpha * 255);
  }
}

// ---------- 组装 PNG ----------
const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(S, 0);
ihdr.writeUInt32BE(S, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // color type: RGBA
const png = Buffer.concat([
  sig,
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

writeFileSync("icon-source.png", png);
console.log("已生成 icon-source.png (" + png.length + " bytes)");
