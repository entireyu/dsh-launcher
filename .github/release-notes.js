// 从 CHANGELOG.md 提取「最新一个带日期的发布章节」作为 Release 说明。
// 输出写入 $GITHUB_OUTPUT 的 body 变量（多行 heredoc 语法），供工作流
// tauri-action 的 releaseBody 使用。两个平台的 runner 都预装 Node，无需额外依赖。
const fs = require("fs");
const t = fs.readFileSync("CHANGELOG.md", "utf8");

// 第一个带日期的章节头，如 "## [0.4.0] - 2026-08-16"
const m = t.match(/\n## \[\d[\d.]*\][^\n]*\n/);
if (!m) {
  console.error("no dated changelog section found");
  process.exit(1);
}
const start = m.index + 1; // 去掉前导换行
const end = t.indexOf("\n## ", start); // 下一个 "## " 行（下一章节）之前截止
const body = (end === -1 ? t.slice(start) : t.slice(start, end)).trim();
if (!body) {
  console.error("empty changelog section");
  process.exit(1);
}
fs.appendFileSync(process.env.GITHUB_OUTPUT, "body<<EOF\n" + body + "\nEOF\n");
console.log("release notes extracted:", body.length, "chars");
