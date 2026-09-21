#!/usr/bin/env node
// Postinstall: verify the bundled binary and the prebuilt simulator runner,
// then link. Nothing compiles and nothing downloads — a miss fails closed
// with the manual remedy.

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const ROOT = __dirname;
const BIN = path.join(ROOT, "bin", "agent-mobile");
const RUNNER = path.join(ROOT, "runner");

function fail(step, remedy) {
  console.error(`agent-mobile postinstall: ${step}`);
  console.error(`next: ${remedy}`);
  process.exit(1);
}

if (!fs.existsSync(BIN)) {
  fail(
    `bundled binary missing at ${BIN}`,
    "reinstall from a tarball that ships bin/agent-mobile (release builds stage it via scripts/sync-npm-version.sh)",
  );
}

fs.chmodSync(BIN, 0o755);

const probe = spawnSync(BIN, ["--version"], { encoding: "utf8" });
if (probe.status !== 0) {
  fail(
    `bundled binary did not run: ${probe.stderr || probe.error?.message || "unknown"}`,
    "this package ships a macOS arm64 binary; on a mismatched host install from source instead",
  );
}

let hasRunner = false;
try {
  hasRunner = fs.readdirSync(RUNNER).some((f) => f.endsWith(".xctestrun"));
} catch {}
if (!hasRunner) {
  fail(
    `bundled simulator runner missing under ${RUNNER}`,
    "the tarball must ship runner/*.xctestrun; set AGENT_MOBILE_DRIVER_DIR to a source checkout to run anyway",
  );
}

console.log(`agent-mobile ${probe.stdout.trim()} ready — run \`agent-mobile skills\` for the agent guide`);
