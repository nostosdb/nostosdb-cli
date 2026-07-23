# Release-candidate tooling

This directory builds and verifies candidate artifacts for the targets in
`release-manifest.json`. The npm `0.0.1` packages assembled from this source
were separately authorized and published; direct archives remain unpublished.
Every archive requires target-native smoke evidence and includes the CLI,
repository notices, a deterministic SPDX inventory, and third-party attribution
metadata.

The scripts do not publish, tag, sign with production credentials, create a Homebrew tap, or contact a package registry except when an operator explicitly runs npm tooling. This repository contains no GitHub Actions; candidate execution and any separately authorized provenance attestation are operator-owned.

Typical target-native sequence:

```bash
python3 distribution/scripts/generate_metadata.py --output dist/metadata
python3 distribution/scripts/smoke_candidate.py \
  --target TARGET --binary target/TARGET/release/nostdb --output dist/native.json
python3 distribution/scripts/assemble_candidate.py \
  --target TARGET --binary target/TARGET/release/nostdb \
  --native-evidence dist/native.json --metadata dist/metadata --output dist
python3 distribution/scripts/verify_candidate.py --archive dist/ARCHIVE
python3 distribution/scripts/stage_npm_candidate.py \
  --target TARGET --binary target/TARGET/release/nostdb --output dist/npm
```

Release-formula rendering requires both verified macOS archives. For local native
verification, `render_homebrew.py --host-smoke-archive ARCHIVE` emits an explicitly
non-publishable formula whose two architecture branches reference the same verified
host archive; this exists only to exercise installation on that native host. The
release formula still requires distinct Apple Silicon and Intel evidence.
