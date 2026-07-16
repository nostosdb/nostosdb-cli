# Agent instructions for nostos-cli

Follow the Root `AGENTS.md` when working in the multi-repository workspace.

- This repository owns command-line UX only and consumes the public `nostos-engine` facade.
- Do not duplicate parsing, storage, synchronization, or query behavior.
- Stage 0 permits only the compiling help/version skeleton; Stage 7 behavior is deferred.
- Use stable Rust and Edition 2024.
- Preserve the SSPL-1.0 source-available license assignment.
