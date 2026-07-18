// SPDX-License-Identifier: SSPL-1.0

"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { spawn } = require("node:child_process");
const { resolveBinary } = require("./platform.cjs");

function launcherVersion() {
  const manifest = JSON.parse(
    fs.readFileSync(path.join(__dirname, "..", "package.json"), "utf8"),
  );
  return manifest.version;
}

function launchBinary(binary, arguments_, options = {}) {
  const child = spawn(binary, arguments_, {
    stdio: options.stdio || "inherit",
    env: options.env || process.env,
    windowsHide: false,
  });
  const forwarded = new Map();
  for (const signalName of ["SIGINT", "SIGTERM"]) {
    const handler = () => {
      if (child.exitCode === null && child.signalCode === null) {
        child.kill(signalName);
      }
    };
    process.on(signalName, handler);
    forwarded.set(signalName, handler);
  }
  const cleanup = () => {
    for (const [signalName, handler] of forwarded) {
      process.removeListener(signalName, handler);
    }
  };
  child.on("error", (error) => {
    cleanup();
    console.error(`nostos launcher: cannot execute ${binary}: ${error.message}`);
    process.exitCode = 3;
  });
  child.on("exit", (code, signalName) => {
    cleanup();
    if (signalName && process.platform !== "win32") {
      process.kill(process.pid, signalName);
      return;
    }
    process.exitCode = code === null ? 3 : code;
  });
  return child;
}

function run(arguments_) {
  try {
    const version = launcherVersion();
    const binary = resolveBinary({ version });
    return launchBinary(binary, arguments_);
  } catch (error) {
    console.error(`nostos launcher: ${error.message}`);
    process.exitCode = 3;
    return null;
  }
}

module.exports = { launchBinary, launcherVersion, run };
