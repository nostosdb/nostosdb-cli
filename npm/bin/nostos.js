#!/usr/bin/env node
// SPDX-License-Identifier: SSPL-1.0

"use strict";

const { run } = require("../lib/launcher.cjs");

run(process.argv.slice(2));
