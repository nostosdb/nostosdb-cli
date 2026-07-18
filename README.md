# nostos-cli

Source-available command-line consumer of the NostosDB engine, licensed under SSPL-1.0.

Public-preview source only: no supported binary, installer, stable database format, or external contribution intake exists. See [PREVIEW.md](PREVIEW.md), [SECURITY.md](SECURITY.md), and [CLA status](CLA.md).

The [distribution contract](DISTRIBUTION.md) provides the same Core-containing `nostos` executable through npm, pinned `npx` zero-install execution, Homebrew, and direct archives. Candidate launchers, packages, archives, and workflows are implemented, but none of those channels is published.

The CLI provides one-shot, query-file, piped-stdin, and interactive query modes. Embedded and Source Mode call the public `nostos-engine` facade. Server Mode uses the thin `nostos-client` protocol crate and never opens daemon-managed files or depends on HTTP endpoints. It also provides synchronization, integrity checks, inspection, project diagnostics, graph statistics, and named Database administration.

```bash
nostos query 'MATCH (n:Person) RETURN n.name' --database graph.ndb
nostos query --file report.cypher --database graph.ndb --format json
cat report.cypher | nostos query --database graph.ndb --format jsonl
nostos query --database graph.ndb
nostos format --file graph.nostos
nostos format --file graph.nostos --check
```

With an explicitly installed `nostosd`:

```bash
nostos server ping --server nostos://127.0.0.1:7878 \
  --credential-file /var/lib/nostosdb/credentials/client.token
nostos database create knowledge --server nostos://127.0.0.1:7878 \
  --credential-file /var/lib/nostosdb/credentials/admin.token
nostos query 'MATCH (n) RETURN n' --server nostos://127.0.0.1:7878 \
  --database knowledge \
  --credential-file /var/lib/nostosdb/credentials/client.token
```

Database commands include create/list/inspect/rename, exact-name-confirmed drop, physical snapshot/restore, and logical export/import. The remote REPL supports `:ping`, `:begin`, `:commit`, and `:rollback`. Credentials come only from `NOSTOS_CREDENTIAL` or `--credential-file`; there is no credential command-line value.

`format` sends one complete source file through the public Core canonical formatter. By default it writes the formatted source to stdout without changing the input; `--check` returns project exit code 3 when the input is not already canonical. Use `--project` to read `language_version` from `nostos.toml`, or supply `--language-version` explicitly.

Standalone `schema`, `unresolved`, `imports`, and `warnings` commands expose the same administration data as the REPL in table or JSON form for scripts and Agent Skills.

Interactive statements end with `;` and may span lines. Administrative commands include `:status`, `:sync`, `:schema`, `:warnings`, `:imports`, `:unresolved`, and explicit NDB-only transactions. Source Mode queries use `--project`; writes additionally require `--owner MODULE_ID` and are routed through the content-hash-guarded source writer.

Machine formats are `json`, `jsonl`, and `csv`; prompts and diagnostics are written to stderr. Stable exit codes are 0 (success), 2 (usage), 3 (project/configuration), 4 (query), 5 (database/integrity), 6 (source conflict), and 7 (I/O).

The local development manifest depends on sibling `nostos-engine` and the sibling Server's Core-free `nostos-client` crate by path and compatible version. It does not access Core storage internals or Server daemon internals.

## Verify

```bash
cargo metadata --no-deps
cargo fmt --all --check
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run -- --help
cargo run -- --version
python3 -m unittest discover -s distribution/tests -v
npm test --prefix npm
npm run verify --prefix npm
python3 distribution/scripts/verify_local.py --skills-root ../nostos-skills
```

## License

Source-available under SSPL-1.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
