/**
 * Assemble the npm packages for one release from its GitHub release assets.
 *
 * Usage: node npm/build.mjs <version> <assets-dir> <out-dir>
 *
 *   version     the release version without the `v`, e.g. 0.5.4
 *   assets-dir  where the release's `.tar.gz` / `.zip` files were downloaded
 *   out-dir     where to write the packages (wiped first)
 *
 * It writes six packages under out-dir, ready for `npm publish`: five platform packages
 * (each holding one binary, restricted by `os`/`cpu`) and the `whoisport` launcher that
 * depends on all of them as optional dependencies. Run by the `npm` job in
 * .github/workflows/release.yml.
 */
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
/** Fields every package shares. */
const common = {
  version,
  license: "MIT",
  repository: { type: "git", url: `git+${repo}.git` },
  homepage: repo,
};
/** npm package suffix -> [Rust target triple, npm `os`, npm `cpu`]. */
const platforms = {
  "linux-x64": ["x86_64-unknown-linux-musl", "linux", "x64"],
  "linux-arm64": ["aarch64-unknown-linux-musl", "linux", "arm64"],
  "darwin-x64": ["x86_64-apple-darwin", "darwin", "x64"],
  "darwin-arm64": ["aarch64-apple-darwin", "darwin", "arm64"],
  // Named "windows", not "win32": npm's spam filter rejects the name `whoisport-win32-x64`
  // (E403). The `os` field below is still npm's own "win32".
  "windows-x64": ["x86_64-pc-windows-msvc", "win32", "x64"],
};

// One package per platform: unpack that platform's release asset and keep only the binary.
/**
 * The README of a platform package. People land on these pages from `npm ls` or a search, so
 * it says what the package is and sends them to the one they should install.
 */
function platformReadme(name, os, cpu) {
  // npm's identifiers ("win32", "x64") are not how people name their systems.
  const system = { win32: "Windows", darwin: "macOS", linux: "Linux" }[os] ?? os;
  const arch = { x64: "x86-64", arm64: "ARM64" }[cpu] ?? cpu;
  return `# ${name}

The [whoisport](https://github.com/dpologdvinity/whoisport) command-line program for
${system} (${arch}). **You do not install this package yourself.**

whoisport shows what is listening on a TCP or UDP port, who started it, and can stop it
safely. Install the main package instead; it pulls in the matching binary for your
system automatically:

\`\`\`sh
npm install -g whoisport    # or: pnpm add -g whoisport, yarn global add whoisport, bun add -g whoisport
whoisport 3000              # what is on port 3000?
\`\`\`

- Documentation: https://github.com/dpologdvinity/whoisport#readme
- Issues: https://github.com/dpologdvinity/whoisport/issues
- License: MIT
`;
}

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
  // npm shows a package's README on its page; without one the page says "no README".
  writeFileSync(join(dir, "README.md"), platformReadme(`whoisport-${suffix}`, os, cpu));
  copyFileSync(new URL("../LICENSE", import.meta.url), join(dir, "LICENSE"));
}

// The launcher package: no binary, just bin/whoisport.js and optional dependencies on the
// five platform packages at exactly this version.
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
