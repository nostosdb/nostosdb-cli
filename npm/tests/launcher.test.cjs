// SPDX-License-Identifier: SSPL-1.0

"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { spawn } = require("node:child_process");

const launcherPath = path.resolve(__dirname, "..", "lib", "launcher.cjs");

function runHelper(source, environment = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ["-e", source], {
      env: { ...process.env, ...environment },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (data) => {
      stdout += data;
    });
    child.stderr.on("data", (data) => {
      stderr += data;
    });
    child.on("error", reject);
    child.on("exit", (code, signal) => resolve({ code, signal, stderr, stdout }));
  });
}

test("forwards exact arguments, stdout, stderr, and exit status", async (context) => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "nostdb-launcher-test-"));
  context.after(() => fs.rmSync(temporary, { recursive: true, force: true }));
  const childScript = path.join(temporary, "child.cjs");
  fs.writeFileSync(
    childScript,
    "console.log(JSON.stringify(process.argv.slice(2)));\n" +
      "console.error('child stderr');\n" +
      "process.exitCode = 7;\n",
  );
  const source =
    `const { launchBinary } = require(${JSON.stringify(launcherPath)});` +
    `launchBinary(process.execPath, [${JSON.stringify(childScript)}, ` +
    `"value with spaces", ";not-shell"]);`;
  const result = await runHelper(source);
  assert.equal(result.code, 7);
  assert.equal(result.signal, null);
  assert.equal(result.stdout, '["value with spaces",";not-shell"]\n');
  assert.equal(result.stderr, "child stderr\n");
});

test(
  "forwards SIGTERM to the native CLI",
  { skip: process.platform === "win32" },
  async (context) => {
    const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "nostdb-signal-test-"));
    context.after(() => fs.rmSync(temporary, { recursive: true, force: true }));
    const signalFile = path.join(temporary, "signal.txt");
    const childScript = path.join(temporary, "signal-child.cjs");
    fs.writeFileSync(
      childScript,
      "const fs = require('node:fs');\n" +
        "process.on('SIGTERM', () => {\n" +
        "  fs.writeFileSync(process.env.SIGNAL_FILE, 'SIGTERM');\n" +
        "  process.exit(0);\n" +
        "});\n" +
        "console.log('ready');\nsetInterval(() => {}, 1000);\n",
    );
    const source =
      `const { launchBinary } = require(${JSON.stringify(launcherPath)});` +
      `launchBinary(process.execPath, [${JSON.stringify(childScript)}]);`;
    const parent = spawn(process.execPath, ["-e", source], {
      env: { ...process.env, SIGNAL_FILE: signalFile },
      stdio: ["ignore", "pipe", "pipe"],
    });
    await new Promise((resolve, reject) => {
      parent.stdout.once("data", resolve);
      parent.once("error", reject);
    });
    parent.kill("SIGTERM");
    const result = await new Promise((resolve, reject) => {
      parent.once("error", reject);
      parent.once("exit", (code, signal) => resolve({ code, signal }));
    });
    assert.deepEqual(result, { code: 0, signal: null });
    assert.equal(fs.readFileSync(signalFile, "utf8"), "SIGTERM");
  },
);
