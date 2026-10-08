import { defineConfig } from "vite";
import { execFileSync } from "node:child_process";
import { VERSION } from "./src/generated/version";

const repository = new URL("../../", import.meta.url).pathname;
let commit = process.env.GITHUB_SHA || "unknown";
let dirty = true;
try {
  commit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: repository,
    encoding: "utf8",
  }).trim();
  dirty =
    execFileSync("git", ["status", "--porcelain", "--untracked-files=no"], {
      cwd: repository,
      encoding: "utf8",
    }).trim().length > 0;
} catch {
  /* 导出的源码包没有 Git 时，不伪造提交信息。 */
}

export default defineConfig({
  root: new URL(".", import.meta.url).pathname,
  base: "./",
  define: {
    __APP_VERSION__: JSON.stringify(VERSION),
    __BUILD_INFO__: JSON.stringify({ version: VERSION, commit, dirty }),
  },
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  build: { outDir: "../dist", emptyOutDir: true, target: "es2022" },
});
