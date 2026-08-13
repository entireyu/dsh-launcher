import { readFileSync } from "node:fs";
import { inflateSync } from "node:zlib";

const buf = readFileSync("icon-source.png");
const w = buf.readUInt32BE(16);
const h = buf.readUInt32BE(20);
const bitDepth = buf[24];
const colorType = buf[25];
console.log(`IHDR: ${w}x${h} bitDepth=${bitDepth} colorType=${colorType}`);

let off = 8;
const idat = [];
while (off < buf.length) {
  const clen = buf.readUInt32BE(off);
  const type = buf.toString("ascii", off + 4, off + 8);
  const data = buf.subarray(off + 8, off + 8 + clen);
  if (type === "IDAT") idat.push(data);
  off += 12 + clen;
}
const raw = inflateSync(Buffer.concat(idat));
const px = (x, y) => {
  const i = y * (1 + w * 4) + 1 + x * 4;
  return [raw[i], raw[i + 1], raw[i + 2], raw[i + 3]];
};
const fmt = (p) => `rgba(${p[0]},${p[1]},${p[2]},${p[3]})`;
console.log("corner(3,3)      ", fmt(px(3, 3)), "  期望 alpha=0");
console.log("center(512,512)  ", fmt(px(512, 512)), "  期望白(三角)");
console.log("ring-top(512,248)", fmt(px(512, 248)), "  期望白(圆环)");
console.log("bg(512,150)      ", fmt(px(512, 150)), "  期望蓝(背景)");
console.log("inside(512,320)  ", fmt(px(512, 320)), "  期望蓝(环内空白)");
