# CLI preview status

The CLI is source-available SSPL-1.0 evaluation software with no supported binary release.

- Commands, exit codes, and machine formats are implemented but remain pre-release surfaces.
- Format 0 databases have no stable byte compatibility.
- Source Mode writes remain one-owner, content-hash guarded, canonical rewrites followed by sync.
- Remote CLI/REPL and named Database administration use database protocol version 1. That preview protocol has no TLS negotiation and must remain on loopback or a trusted container network.
- No installer, shell completion package, production support, or contribution intake exists.

Use disposable projects and run [README verification](README.md#verify) before evaluation.
