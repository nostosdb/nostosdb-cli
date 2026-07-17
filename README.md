# nostos-cli

Source-available command-line consumer of the NostosDB engine, licensed under SSPL-1.0.

Public-preview source only: no supported binary, installer, stable database format, or external contribution intake exists. See [PREVIEW.md](PREVIEW.md), [SECURITY.md](SECURITY.md), and [CLA status](CLA.md).

The CLI provides one-shot, query-file, piped-stdin, and interactive query modes over the public `nostos-engine` facade. It also provides synchronization, integrity checks, inspection, project diagnostics, and graph statistics.

```bash
nostos query 'MATCH (n:Person) RETURN n.name' --database graph.ndb
nostos query --file report.cypher --database graph.ndb --format json
cat report.cypher | nostos query --database graph.ndb --format jsonl
nostos query --database graph.ndb
nostos format --file graph.nostos
nostos format --file graph.nostos --check
```

`format` sends one complete source file through the public Core canonical formatter. By default it writes the formatted source to stdout without changing the input; `--check` returns project exit code 3 when the input is not already canonical. Use `--project` to read `language_version` from `nostos.toml`, or supply `--language-version` explicitly.

Standalone `schema`, `unresolved`, `imports`, and `warnings` commands expose the same administration data as the REPL in table or JSON form for scripts and Agent Skills.

Interactive statements end with `;` and may span lines. Administrative commands include `:status`, `:sync`, `:schema`, `:warnings`, `:imports`, `:unresolved`, and explicit NDB-only transactions. Source Mode queries use `--project`; writes additionally require `--owner MODULE_ID` and are routed through the content-hash-guarded source writer.

Machine formats are `json`, `jsonl`, and `csv`; prompts and diagnostics are written to stderr. Stable exit codes are 0 (success), 2 (usage), 3 (project/configuration), 4 (query), 5 (database/integrity), 6 (source conflict), and 7 (I/O).

The local development manifest depends on sibling `nostos-engine` by path and compatible version. It does not access Core storage internals.

## Verify

```bash
cargo metadata --no-deps
cargo fmt --all --check
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run -- --help
cargo run -- --version
```

## License

Source-available under SSPL-1.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
