/**
 * Consistency check for the npm packaging, run by CI (`node npm/check.mjs`).
 *
 * The launcher (`whoisport/bin/whoisport.js`) names the platform package it will `require`,
 * and `build.mjs` decides which packages exist and what they are called. If the two ever
 * disagree (for example one is renamed and the other is not), installs on that platform
 * silently break, so this fails the build instead. It also pins each platform to the Rust
 * target whose release asset it is built from.
 */
import { readFileSync } from "node:fs";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
const launcher = read("./whoisport/bin/whoisport.js");
const build = read("./build.mjs");

// `"darwin arm64": "whoisport-darwin-arm64",` in the launcher: the names it can load.
const launcherNames = [...launcher.matchAll(/"(\w+) (\w+)":\s*"(whoisport-[\w-]+)"/g)].map(
  ([, platform, arch, name]) => ({ platform, arch, name }),
);
// `"windows-x64": ["x86_64-pc-windows-msvc", "win32", "x64"],` in build.mjs: the packages built.
const built = [...build.matchAll(/"([\w-]+)":\s*\["([\w-]+)",\s*"(\w+)",\s*"(\w+)"\]/g)].map(
  ([, suffix, , os, cpu]) => ({ name: `whoisport-${suffix}`, os, cpu }),
);

const problems = [];
if (launcherNames.length === 0 || built.length === 0) {
  problems.push("could not read the platform tables (did the file format change?)");
}
for (const l of launcherNames) {
  const match = built.find((b) => b.name === l.name);
  if (!match) problems.push(`launcher loads ${l.name}, but build.mjs does not build it`);
  else if (match.os !== l.platform || match.cpu !== l.arch) {
    problems.push(`${l.name}: launcher says ${l.platform}/${l.arch}, build.mjs says ${match.os}/${match.cpu}`);
  }
}
for (const b of built) {
  if (!launcherNames.some((l) => l.name === b.name)) {
    problems.push(`build.mjs builds ${b.name}, but the launcher never loads it`);
  }
}

if (problems.length > 0) {
  console.error(problems.map((p) => `npm check: ${p}`).join("\n"));
  process.exit(1);
}
console.log(`npm check: ${built.length} platform packages, launcher and build agree`);
