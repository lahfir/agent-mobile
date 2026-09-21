#!/usr/bin/env node
// Thin bin shim: spawn the bundled binary and forward everything.
// No logic worth testing lives here — arguments, stdio, signals, and the
// exit code pass through untouched.

const { spawnSync } = require("node:child_process");
const path = require("node:path");

const BIN = path.join(__dirname, "bin", "agent-mobile");

function manualRemedy() {
  console.error(
    "agent-mobile: the bundled binary is missing.\n" +
      `next: run \`node ${path.join(__dirname, "install.js")}\` once, ` +
      "or reinstall without --ignore-scripts",
  );
  process.exit(1);
}

const r = spawnSync(BIN, process.argv.slice(2), { stdio: "inherit" });
if (r.error) {
  if (r.error.code === "ENOENT") manualRemedy();
  console.error(`agent-mobile: ${r.error.message}`);
  process.exit(1);
}
if (r.signal) {
  process.kill(process.pid, r.signal);
} else {
  process.exit(r.status === null ? 1 : r.status);
}
