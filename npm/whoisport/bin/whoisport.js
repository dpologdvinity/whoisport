#!/usr/bin/env node
"use strict";
// Runs the prebuilt whoisport binary from the platform package that npm (or pnpm,
// yarn, bun) installed as an optional dependency.
const { spawnSync } = require("node:child_process");

const packages = {
  "linux x64": "whoisport-linux-x64",
  "linux arm64": "whoisport-linux-arm64",
  "darwin x64": "whoisport-darwin-x64",
  "darwin arm64": "whoisport-darwin-arm64",
  "win32 x64": "whoisport-win32-x64",
};
const name = packages[`${process.platform} ${process.arch}`];
if (!name) {
  console.error(`whoisport: no prebuilt binary for ${process.platform}-${process.arch}`);
  process.exit(1);
}
const exe = process.platform === "win32" ? "whoisport.exe" : "whoisport";
let bin;
try {
  bin = require.resolve(`${name}/bin/${exe}`);
} catch {
  console.error(`whoisport: ${name} is not installed (optional dependencies skipped?)`);
  process.exit(1);
}
const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`whoisport: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) process.kill(process.pid, result.signal);
process.exit(result.status ?? 1);
