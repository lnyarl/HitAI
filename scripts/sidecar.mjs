// 훅 바이너리를 빌드해서 Tauri 사이드카 위치로 복사한다.
// Tauri는 `binaries/hitai-hook-<target-triple>` 형태의 파일을 요구한다.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const profile = process.argv.includes("--release") ? "release" : "debug";

const triple = execFileSync("rustc", ["-vV"], { encoding: "utf8" })
  .split("\n")
  .find((line) => line.startsWith("host:"))
  .replace("host:", "")
  .trim();

const ext = process.platform === "win32" ? ".exe" : "";

const args = ["build", "-p", "hitai-hook"];
if (profile === "release") args.push("--release");
execFileSync("cargo", args, { cwd: root, stdio: "inherit" });

const src = join(root, "target", profile, `hitai-hook${ext}`);
if (!existsSync(src)) {
  console.error(`훅 바이너리를 찾지 못했습니다: ${src}`);
  process.exit(1);
}

const outDir = join(root, "src-tauri", "binaries");
mkdirSync(outDir, { recursive: true });
const dst = join(outDir, `hitai-hook-${triple}${ext}`);
copyFileSync(src, dst);
console.log(`사이드카 준비 완료: ${dst}`);
