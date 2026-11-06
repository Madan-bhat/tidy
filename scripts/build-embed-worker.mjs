#!/usr/bin/env node
/**
 * Build the ONNX sidecar crate and place it where Tauri `externalBin` expects:
 *   src-tauri/binaries/tidy-embed-worker-<target-triple>
 */
import { execSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const tauri = join(root, "src-tauri");
const workerCrate = join(tauri, "tidy-embed-worker");
const binaries = join(tauri, "binaries");
const release = process.env.TIDY_EMBED_PROFILE !== "debug";

const triple = execSync("rustc -vV", { encoding: "utf8" })
  .split("\n")
  .find((l) => l.startsWith("host:"))
  ?.replace("host:", "")
  .trim();
if (!triple) {
  console.error("Could not detect rustc host triple");
  process.exit(1);
}

mkdirSync(binaries, { recursive: true });
const destName =
  process.platform === "win32"
    ? `tidy-embed-worker-${triple}.exe`
    : `tidy-embed-worker-${triple}`;
const dest = join(binaries, destName);

// Ensure a placeholder exists so `cargo check` / tauri-build can resolve externalBin
// before this script has finished (or when only the UI crate is built).
if (!existsSync(dest)) {
  if (process.platform === "win32") {
    writeFileSync(dest, "");
  } else {
    writeFileSync(
      dest,
      '#!/bin/sh\necho \'{"ready":false,"error":"embed worker stub"}\'\n',
    );
    chmodSync(dest, 0o755);
  }
}

const cargoArgs = [
  "build",
  ...(release ? ["--release"] : []),
  "--manifest-path",
  join(workerCrate, "Cargo.toml"),
];
console.log(
  `Building tidy-embed-worker (${release ? "release" : "debug"}, ${triple})…`,
);
execSync(`cargo ${cargoArgs.join(" ")}`, {
  cwd: workerCrate,
  stdio: "inherit",
  env: process.env,
});

const builtName =
  process.platform === "win32" ? "tidy-embed-worker.exe" : "tidy-embed-worker";
const targetDir =
  process.env.CARGO_TARGET_DIR ||
  join(workerCrate, "target");
const built = join(targetDir, release ? "release" : "debug", builtName);
if (!existsSync(built)) {
  console.error(`Missing built worker at ${built}`);
  process.exit(1);
}

copyFileSync(built, dest);
copyFileSync(built, join(binaries, builtName));
if (process.platform !== "win32") {
  chmodSync(dest, 0o755);
  chmodSync(join(binaries, builtName), 0o755);
}
console.log(`Wrote ${dest}`);
