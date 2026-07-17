# Agent instructions for nostos-cli

Follow the Root `AGENTS.md` when working in the multi-repository workspace.

- This repository owns command-line UX only and consumes the public `nostos-engine` facade.
- Do not duplicate parsing, storage, synchronization, or query behavior.
- Keep one-shot, file/stdin, and REPL queries on the same public Engine execution surfaces.
- Keep prompts and progress messages on stderr so JSON, JSONL, and CSV stdout remains clean.
- Source Mode writes must use `SourceWriter`; never mutate a synchronized `.ndb` directly.
- Use stable Rust and Edition 2024.
- Preserve the SSPL-1.0 source-available license assignment.
