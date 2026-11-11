import { build } from "esbuild";
import { cpSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
mkdirSync("../docs", { recursive: true });
await build({ entryPoints: ["src/main.js"], bundle: true, minify: true, format: "esm", target: "es2022",
  outfile: "../docs/app.js", define: { global: "globalThis", "process.env.NODE_ENV": '"production"' }, loader: { ".css": "text" }, platform: "browser", inject: ["./shims.js"] });
const modalCss = readFileSync("node_modules/@near-wallet-selector/modal-ui/styles.css", "utf8");
writeFileSync("../docs/app.css", modalCss + "\n" + readFileSync("app.css", "utf8"));
cpSync("index.html", "../docs/index.html");
writeFileSync("../docs/.nojekyll", "");
console.log("built ../docs");
