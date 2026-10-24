#!/usr/bin/env node
"use strict";
/**
 * whoisport launcher.
 *
 * The `whoisport` npm package contains no binary of its own. Each platform's binary lives
 * in its own package (`whoisport-linux-x64`, `whoisport-win32-x64`, ...), listed as an
 * optional dependency with `os`/`cpu` set, so npm, pnpm, yarn and bun install only the one
 * that matches this machine. This script finds that package, runs its binary with the same
 * arguments, and passes the exit code (or terminating signal) straight through.
 */
const { spawnSync } = require("node:child_process");

/** Platform + architecture, as `process.platform process.arch`, to npm package name. */
const packages = {
  "linux x64": "whoisport-linux-x64",
  "linux arm64": "whoisport-linux-arm64",
  "darwin x64": "whoisport-darwin-x64",
  "darwin arm64": "whoisport-darwin-arm64",
  "win32 x64": "whoisport-win32-x64",
};

/** Print `message` to stderr and exit with failure. */
function fail(message) {
  console.error(`whoisport: ${message}`);
  process.exit(1);
}

const name = packages[`${process.platform} ${process.arch}`];
if (!name) fail(`no prebuilt binary for ${process.platform}-${process.arch}`);

const exe = process.platform === "win32" ? "whoisport.exe" : "whoisport";
let bin;
try {
  // `require.resolve` finds the binary inside the platform package's install directory.
  bin = require.resolve(`${name}/bin/${exe}`);
} catch {
  fail(`${name} is not installed (were optional dependencies skipped?)`);
}

const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (result.error) fail(result.error.message);
// Re-raise the child's signal so the shell sees the same cause of death.
if (result.signal) process.kill(process.pid, result.signal);
process.exit(result.status ?? 1);
