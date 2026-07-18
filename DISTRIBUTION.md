# CLI distribution contract

Status: candidate tooling implemented; no package, installer, or supported binary is published.

NostosDB distributes one executable named `nostos`. The Rust binary includes `nostos-engine` and its Core dependencies at build time. Core is not a second user-installed package, daemon, or executable.

The first distribution implementation must produce the same CLI version and behavior through:

- `npm install --global @nostosdb/cli@VERSION`;
- `npx --yes --package=@nostosdb/cli@VERSION nostos ...`;
- `brew install nostosdb/tap/nostos`; and
- signed/checksummed `.tar.gz` or `.zip` release archives.

The target matrix is macOS 13+ on Apple Silicon and Intel, Windows 10/11 on x64 and ARM64, and Ubuntu 22.04/24.04 LTS on x64 and ARM64. A target must pass native smoke and conformance tests before it is advertised. Linux initially uses the declared GNU/glibc baseline; musl is a separate future artifact.

The npm launcher is a thin command dispatcher. It selects an exact-version OS/CPU platform package, invokes its packaged `nostos` executable without a shell, and preserves arguments, stdio, signals, and exit status. It must not duplicate Core behavior. Platform packages and direct archives carry the CLI's SSPL-1.0 `LICENSE`, `NOTICE`, README, checksum, attestation, and approved third-party attribution material.

All channel artifacts originate from the same reviewed Git tag and release manifest. No existing version may be replaced. Candidate workflows may build and verify packages, but publication remains separately authorized and legally gated.

The candidate source is under [`npm/`](npm/) and [`distribution/`](distribution/). The manual workflow builds all six declared native targets, requires target-native smoke evidence, verifies direct/npm/npx fixture equivalence, attests review archives, and tests the Homebrew formula on Apple Silicon and Intel without publishing any channel. Homebrew 6 local-path installation runs only in its developer verification mode with automatic updates disabled; it creates no tap and the ephemeral runner removes the formula afterward.

See the Root distribution and release policies in a complete NostosDB workspace for the canonical package layout, Skill fallback rules, acceptance criteria, and publication gates.
