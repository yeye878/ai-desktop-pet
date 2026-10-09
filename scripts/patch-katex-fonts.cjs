/**
 * KaTeX 的样式表放在 node_modules/katex/dist/ 下，字体用相对路径 ./fonts/ 引用。
 * 直接从 src/ 里 import 这份 CSS 时，相对路径会被解析到 src/fonts/（不存在），
 * 打包出来的 woff2 就会丢。这里把字体路径改写成 /node_modules/katex/dist/fonts/，
 * 让 Vite 能正确解析、打包并改写资源 URL。
 * 由 package.json 的 postinstall 自动执行，升级 katex 后会重新打补丁。
 */
const fs = require("node:fs");

const FILES = ["node_modules/katex/dist/katex.min.css", "node_modules/katex/dist/katex.css"];
const FROM = "url(fonts/";
const TO = "url(/node_modules/katex/dist/fonts/";

let patched = 0;
for (const file of FILES) {
  if (!fs.existsSync(file)) continue;
  const css = fs.readFileSync(file, "utf8");
  if (css.includes(TO)) continue;
  if (!css.includes(FROM)) continue;
  fs.writeFileSync(file, css.split(FROM).join(TO));
  patched += 1;
  console.log("patched " + file);
}
if (patched === 0) console.log("katex font paths already patched");
