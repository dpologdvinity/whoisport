// Assemble the npm packages for one release from its GitHub release assets.
// Usage: node npm/build.mjs <version> <assets-dir> <out-dir>
import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [version, assets, out] = process.argv.slice(2);
if (!/^\d+\.\d+\.\d+$/.test(version ?? "") || !assets || !out) {
  console.error("usage: node npm/build.mjs <version> <assets-dir> <out-dir>");
  process.exit(2);
}
const repo = "https://github.com/dpologdvinity/whoisport";
const common = {
  version,
  license: "MIT",
  repository: { type: "git", url: `git+${repo}.git` },
  homepage: repo,
};
// npm package suffix -> [Rust target, os, cpu]
const platforms = {
  "linux-x64": ["x86_64-unknown-linux-musl", "linux", "x64"],
  "linux-arm64": ["aarch64-unknown-linux-musl", "linux", "arm64"],
  "darwin-x64": ["x86_64-apple-darwin", "darwin", "x64"],
  "darwin-arm64": ["aarch64-apple-darwin", "darwin", "arm64"],
  "win32-x64": ["x86_64-pc-windows-msvc", "win32", "x64"],
};

rmSync(out, { recursive: true, force: true });
for (const [suffix, [target, os, cpu]] of Object.entries(platforms)) {
  const dir = join(out, `whoisport-${suffix}`);
  const exe = os === "win32" ? "whoisport.exe" : "whoisport";
  const tmp = mkdtempSync(join(tmpdir(), "whoisport-npm-"));
  if (os === "win32") {
    execFileSync("unzip", ["-q", join(assets, `whoisport-v${version}-${target}.zip`), "-d", tmp]);
  } else {
    execFileSync("tar", ["-xzf", join(assets, `whoisport-v${version}-${target}.tar.gz`), "-C", tmp]);
  }
  mkdirSync(join(dir, "bin"), { recursive: true });
  copyFileSync(join(tmp, exe), join(dir, "bin", exe));
  chmodSync(join(dir, "bin", exe), 0o755);
  rmSync(tmp, { recursive: true });
  writeFileSync(
    join(dir, "package.json"),
    JSON.stringify(
      {
        name: `whoisport-${suffix}`,
        description: `whoisport binary for ${os}-${cpu} (installed automatically by the whoisport package)`,
        ...common,
        os: [os],
        cpu: [cpu],
        files: ["bin/"],
        preferUnplugged: true,
      },
      null,
      2,
    ) + "\n",
  );
}

const main = join(out, "whoisport");
mkdirSync(join(main, "bin"), { recursive: true });
copyFileSync(new URL("./whoisport/bin/whoisport.js", import.meta.url), join(main, "bin", "whoisport.js"));
copyFileSync(new URL("../README.md", import.meta.url), join(main, "README.md"));
copyFileSync(new URL("../LICENSE", import.meta.url), join(main, "LICENSE"));
writeFileSync(
  join(main, "package.json"),
  JSON.stringify(
    {
      name: "whoisport",
      description: "Show what is listening on a TCP or UDP port, and optionally kill it",
      ...common,
      keywords: ["port", "process", "cli", "lsof", "devtools", "kill-port"],
      bin: { whoisport: "bin/whoisport.js" },
      files: ["bin/", "README.md", "LICENSE"],
      engines: { node: ">=16" },
      optionalDependencies: Object.fromEntries(
        Object.keys(platforms).map((s) => [`whoisport-${s}`, version]),
      ),
    },
    null,
    2,
  ) + "\n",
);
console.log(`built ${Object.keys(platforms).length + 1} packages in ${out}`);
