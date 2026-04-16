"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const install = require("./install");

test("shell installer dry-run targets GitHub release zip assets", { skip: process.platform === "win32" }, function () {
  const releaseInfo = install.getReleaseAssetInfo(os.platform(), os.arch(), "0.4.2");

  assert.ok(releaseInfo, "expected host platform to be supported by release asset mapping");

  const scriptPath = path.join(__dirname, "..", "scripts", "install.sh");
  const output = childProcess.execFileSync("sh", [scriptPath, "--dry-run"], {
    cwd: path.join(__dirname, ".."),
    env: {
      ...process.env,
      TAMUX_VERSION: "0.4.2",
    },
    encoding: "utf8",
  });

  const expectedUrl = `https://github.com/${install.GITHUB_OWNER}/${install.GITHUB_REPO}/releases/download/v0.4.2/${releaseInfo.archiveName}`;

  assert.match(output, new RegExp(expectedUrl.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.doesNotMatch(output, /gitlab\.com\/api\/v4\/projects/);
  assert.doesNotMatch(output, /tamux-binaries-/);
});

test("shell installer provisions bundled skills into canonical tamux root", function () {
  const scriptPath = path.join(__dirname, "..", "scripts", "install.sh");
  const script = childProcess.execFileSync("sed", ["-n", "1,320p", scriptPath], {
    cwd: path.join(__dirname, ".."),
    encoding: "utf8",
  });

  assert.match(script, /SKILLS_DIR="\$\{TAMUX_SKILLS_DIR:-\$HOME\/\.tamux\/skills\}"/);
  assert.match(script, /Extracting binaries and skills/);
  assert.match(script, /cp -R "\$EXTRACT_DIR\/skills\/\." "\$SKILLS_DIR\/"/);
});

test("shell installer accepts archive-only checksum manifests", { skip: process.platform === "win32" }, function () {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "tamux-install-shell-"));
  const homeDir = path.join(tmpRoot, "home");
  const installDir = path.join(tmpRoot, "bin");
  const skillsDir = path.join(tmpRoot, "skills");
  const mockBinDir = path.join(tmpRoot, "mock-bin");
  const payloadDir = path.join(tmpRoot, "payload");
  const payloadSkillsDir = path.join(payloadDir, "skills", "demo");
  fs.mkdirSync(homeDir, { recursive: true });
  fs.mkdirSync(installDir, { recursive: true });
  fs.mkdirSync(skillsDir, { recursive: true });
  fs.mkdirSync(mockBinDir, { recursive: true });
  fs.mkdirSync(payloadSkillsDir, { recursive: true });

  const binaries = [
    "tamux",
    "tamux-daemon",
    "tamux-tui",
    "tamux-gateway",
    "tamux-mcp",
  ];
  const binaryHashes = new Map();
  for (const name of binaries) {
    const payloadPath = path.join(payloadDir, name);
    const contents = `binary:${name}\n`;
    fs.writeFileSync(payloadPath, contents);
    binaryHashes.set(
      name,
      childProcess.execFileSync("sha256sum", [payloadPath], { encoding: "utf8" }).split(/\s+/)[0]
    );
  }
  fs.writeFileSync(path.join(payloadSkillsDir, "SKILL.md"), "# demo\n");

  const archivePath = path.join(tmpRoot, "tamux-linux-aarch64.zip");
  fs.writeFileSync(archivePath, "mock archive payload\n");
  const archiveHash = childProcess
    .execFileSync("sha256sum", [archivePath], { encoding: "utf8" })
    .split(/\s+/)[0];

  fs.writeFileSync(
    path.join(mockBinDir, "curl"),
    `#!/bin/sh
set -eu
output=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -H)
      shift 2
      ;;
    -o)
      output="$2"
      shift 2
      ;;
    -*)
      shift
      ;;
    *)
      url="$1"
      shift
      ;;
  esac
done

case "$url" in
  *SHA256SUMS-linux-aarch64.txt)
    printf '%s  %s\\n' '${archiveHash}' 'tamux-linux-aarch64.zip' > "$output"
    ;;
  *tamux-linux-aarch64.zip)
    cp '${archivePath}' "$output"
    ;;
  *)
    echo "unexpected mock curl url: $url" >&2
    exit 1
    ;;
esac
`,
    { mode: 0o755 }
  );

  fs.writeFileSync(
    path.join(mockBinDir, "uname"),
    `#!/bin/sh
set -eu
case "\${1:-}" in
  -s) printf 'Linux\\n' ;;
  -m) printf 'aarch64\\n' ;;
  *) /usr/bin/uname "$@" ;;
esac
`,
    { mode: 0o755 }
  );

  fs.writeFileSync(
    path.join(mockBinDir, "unzip"),
    `#!/bin/sh
set -eu
dest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -d)
      dest="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

mkdir -p "$dest"
cp -R '${payloadDir}/.' "$dest/"
`,
    { mode: 0o755 }
  );

  const shaCases = binaries
    .map(function (name) {
      return `  *${name}) printf '%s  %s\\n' '${binaryHashes.get(name)}' "$1" ;;`;
    })
    .join("\n");
  fs.writeFileSync(
    path.join(mockBinDir, "sha256sum"),
    `#!/bin/sh
set -eu
case "$1" in
${shaCases}
  *) /usr/bin/sha256sum "$1" ;;
esac
`,
    { mode: 0o755 }
  );

  const scriptPath = path.join(__dirname, "..", "scripts", "install.sh");
  childProcess.execFileSync("sh", [scriptPath], {
    cwd: path.join(__dirname, ".."),
    env: {
      ...process.env,
      HOME: homeDir,
      PATH: `${mockBinDir}:${process.env.PATH}`,
      TAMUX_VERSION: "0.5.2",
      TAMUX_INSTALL_DIR: installDir,
      TAMUX_SKILLS_DIR: skillsDir,
    },
    encoding: "utf8",
  });

  for (const name of binaries) {
    assert.ok(fs.existsSync(path.join(installDir, name)), `expected ${name} to be installed`);
  }
  assert.ok(fs.existsSync(path.join(skillsDir, "demo", "SKILL.md")));
});
