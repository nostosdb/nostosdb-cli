// SPDX-License-Identifier: SSPL-1.0

"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const {
  PlatformError,
  packageFor,
  resolveBinary,
} = require("../lib/platform.cjs");

const glibcReport = {
  getReport: () => ({ header: { glibcVersionRuntime: "2.35" } }),
};

test("selects every declared OS and CPU package", () => {
  assert.equal(packageFor("darwin", "arm64"), "@nostosdb/cli-darwin-arm64");
  assert.equal(packageFor("darwin", "x64"), "@nostosdb/cli-darwin-x64");
  assert.equal(packageFor("win32", "arm64"), "@nostosdb/cli-win32-arm64");
  assert.equal(packageFor("win32", "x64"), "@nostosdb/cli-win32-x64");
  assert.equal(
    packageFor("linux", "arm64", glibcReport),
    "@nostosdb/cli-linux-arm64-gnu",
  );
  assert.equal(
    packageFor("linux", "x64", glibcReport),
    "@nostosdb/cli-linux-x64-gnu",
  );
});

test("rejects unsupported operating systems, CPUs, and Linux libc", () => {
  assert.throws(() => packageFor("freebsd", "x64"), PlatformError);
  assert.throws(() => packageFor("darwin", "ia32"), PlatformError);
  assert.throws(
    () => packageFor("linux", "x64", { getReport: () => ({ header: {} }) }),
    /GNU\/glibc/,
  );
});

test("requires an exact-version platform package and executable", (context) => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "nostos-platform-test-"));
  context.after(() => fs.rmSync(temporary, { recursive: true, force: true }));
  const packageRoot = path.join(temporary, "package");
  const binaryDirectory = path.join(packageRoot, "bin");
  fs.mkdirSync(binaryDirectory, { recursive: true });
  const manifestPath = path.join(packageRoot, "package.json");
  fs.writeFileSync(
    manifestPath,
    JSON.stringify({ name: "@nostosdb/cli-darwin-arm64", version: "0.1.0" }),
  );
  const binary = path.join(binaryDirectory, "nostos");
  fs.writeFileSync(binary, "fixture");
  const resolvePackage = (request) => {
    assert.equal(request, "@nostosdb/cli-darwin-arm64/package.json");
    return manifestPath;
  };
  assert.equal(
    resolveBinary({
      platform: "darwin",
      arch: "arm64",
      version: "0.1.0",
      resolvePackage,
    }),
    binary,
  );
  assert.throws(
    () =>
      resolveBinary({
        platform: "darwin",
        arch: "arm64",
        version: "0.2.0",
        resolvePackage,
      }),
    /platform package mismatch/,
  );
});
