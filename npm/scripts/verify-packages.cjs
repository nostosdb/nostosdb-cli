// SPDX-License-Identifier: SSPL-1.0

"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { PLATFORM_PACKAGES } = require("../lib/platform.cjs");

const root = path.resolve(__dirname, "..");
const launcher = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const expectedDirectories = Object.values(PLATFORM_PACKAGES)
  .map((name) => name.replace("@nostdb/cli-", ""))
  .sort();
const actualDirectories = fs
  .readdirSync(path.join(root, "packages"), { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name)
  .sort();

assert.deepEqual(actualDirectories, expectedDirectories);
assert.equal(launcher.name, "@nostdb/cli");
assert.equal(launcher.license, "SSPL-1.0");
assert.equal(launcher.bin.nostdb, "bin/nostdb.js");
assert.equal(launcher.scripts.preinstall, undefined);
assert.equal(launcher.scripts.install, undefined);
assert.equal(launcher.scripts.postinstall, undefined);

for (const directory of actualDirectories) {
  const manifestPath = path.join(root, "packages", directory, "package.json");
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  assert.equal(manifest.name, `@nostdb/cli-${directory}`);
  assert.equal(manifest.version, launcher.version);
  assert.equal(launcher.optionalDependencies[manifest.name], launcher.version);
  assert.equal(manifest.license, "SSPL-1.0");
  assert.equal(manifest.scripts, undefined);
  assert.deepEqual(manifest.os, [directory.split("-")[0]]);
}

const runtimeFiles = [
  path.join(root, "bin", "nostdb.js"),
  path.join(root, "lib", "launcher.cjs"),
  path.join(root, "lib", "platform.cjs"),
];
const forbidden = [
  "sqlite",
  "nostdb-parser",
  "nostdb-storage",
  "writeFileSync",
  "writeFile",
  ".ndb",
];
for (const file of runtimeFiles) {
  const source = fs.readFileSync(file, "utf8");
  for (const marker of forbidden) {
    assert.equal(source.includes(marker), false, `${file} contains forbidden ${marker}`);
  }
}

console.log(`verified npm launcher and ${actualDirectories.length} platform manifests`);
