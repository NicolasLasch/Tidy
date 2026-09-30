// Maintainer/developer build only. The installed app never runs a compiler or shell.
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const revision = "a7a98e0fffed794396b3fbad4dcdbbc184963645";
const expected =
  "4d78e6aa4a9124b58dff994416525294fbab2990fe905a640d4cbd26bf563a31";
const cache = path.join(root, "target", "native");
fs.mkdirSync(cache, { recursive: true });
const archive = path.join(cache, "llama.tar.gz");
if (!fs.existsSync(archive)) {
  console.log("Downloading pinned llama.cpp source (build-time only)…");
  const response = await fetch(
    `https://codeload.github.com/ggml-org/llama.cpp/tar.gz/${revision}`,
  );
  if (!response.ok)
    throw new Error(`Source download failed: ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length > 100 * 1024 * 1024)
    throw new Error("Source archive too large");
  fs.writeFileSync(archive, bytes);
}
if (
  createHash("sha256").update(fs.readFileSync(archive)).digest("hex") !==
  expected
)
  throw new Error("llama.cpp source checksum mismatch");
function run(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit", shell: false });
  if (result.error || result.status !== 0)
    throw new Error(`${command} failed: ${result.error ?? result.status}`);
}
const source = path.join(cache, `llama.cpp-${revision}`);
if (!fs.existsSync(source)) run("tar", ["-xzf", archive, "-C", cache]);
const build = path.join(cache, "build");
run("cmake", [
  "-S",
  path.join(root, "native", "inference"),
  "-B",
  build,
  `-DLLAMA_SOURCE_DIR=${source}`,
  "-DCMAKE_BUILD_TYPE=Release",
]);
run("cmake", [
  "--build",
  build,
  "--config",
  "Release",
  "--target",
  "tidy-inference-worker",
  "--parallel",
  "4",
]);
const output = path.join(
  root,
  "apps",
  "desktop",
  "src-tauri",
  "resources",
  "inference",
);
fs.mkdirSync(output, { recursive: true });
const name =
  process.platform === "win32"
    ? "tidy-inference-worker.exe"
    : "tidy-inference-worker";
const binary =
  process.platform === "win32"
    ? path.join(build, "Release", name)
    : path.join(build, name);
fs.copyFileSync(binary, path.join(output, name));
const bytes = fs.readFileSync(binary);
fs.writeFileSync(
  path.join(output, "worker-manifest.json"),
  JSON.stringify(
    {
      revision,
      bytes: bytes.length,
      sha256: createHash("sha256").update(bytes).digest("hex"),
      platform: process.platform,
      arch: process.arch,
    },
    null,
    2,
  ) + "\n",
);
fs.copyFileSync(
  path.join(source, "LICENSE"),
  path.join(output, "llama-LICENSE.txt"),
);
fs.copyFileSync(
  path.join(root, "native", "inference", "nlohmann-LICENSE.txt"),
  path.join(output, "nlohmann-LICENSE.txt"),
);
console.log(`Built pinned worker: ${path.join(output, name)}`);
