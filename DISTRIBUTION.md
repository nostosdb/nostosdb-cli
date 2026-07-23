# CLI distribution contract

Status: npm `0.0.1` is published under `latest` and `next`; no supported binary,
Homebrew formula, or direct archive is published.

NostDB distributes one executable named `nostdb`. The Rust binary includes `nostdb-engine` and its Core dependencies at build time. Core is not a second user-installed package, daemon, or executable.

The first distribution implementation must produce the same CLI version and behavior through:

- `npm install --global @nostdb/cli@VERSION`;
- `npx --yes --package=@nostdb/cli@VERSION nostdb ...`;
- `brew install nostdb/tap/nostdb` (the combined CLI and daemon formula); and
- signed/checksummed `.tar.gz` or `.zip` release archives.

The target matrix is macOS 13+ on Apple Silicon and Intel, Windows 10/11 on x64 and ARM64, and Ubuntu 22.04/24.04 LTS on x64 and ARM64. A target must pass native smoke and conformance tests before it is advertised. Linux initially uses the declared GNU/glibc baseline; musl is a separate future artifact.

The npm launcher is a thin command dispatcher. It selects an exact-version OS/CPU platform package, invokes its packaged `nostdb` executable without a shell, and preserves arguments, stdio, signals, and exit status. It must not duplicate Core behavior. Platform packages and direct archives carry the CLI's SSPL-1.0 `LICENSE`, `NOTICE`, README, checksum, attestation, and approved third-party attribution material.

All channel artifacts originate from the same reviewed Git tag and release manifest. No existing version may be replaced. Candidate tools may build and verify packages, but publication remains separately authorized and legally gated.

The candidate source is under [`npm/`](npm/) and [`distribution/`](distribution/). The checked-in tools cover all six declared native targets, require target-native smoke evidence, verify direct/npm/npx fixture equivalence, and retain the historical CLI-only Homebrew candidate test without publishing any channel. This repository contains no GitHub Actions, so operators must run those tools in separately controlled target-native environments and arrange any required attestation through an explicitly reviewed process. The public formula contract is the combined `nostdb` template in `nostdb-server`; the older local `nostdb.rb` template exists only for Stage 13 evidence and must not be published. Homebrew 6 local-path installation runs only in its developer verification mode with automatic updates disabled; it creates no tap and the operator removes the formula afterward.

The published `@nostdb/server@0.0.1` global package belongs to the Server
distribution and depends on this exact matching `@nostdb/cli@0.0.1` package. It
exposes both `nostd` and `nostdb`; this repository continues to own only the CLI
launcher and `nostdb` platform packages. No `@nostdb/core` npm package is part
of either installation.

See the Root distribution and release policies in a complete NostDB workspace for the canonical package layout, Skill fallback rules, acceptance criteria, and publication gates.
