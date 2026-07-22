# nostosdb-cli

Source-available command-line consumer of the NostosDB engine, licensed under SSPL-1.0.

Public-preview source only: no supported binary, installer, stable database format, or external contribution intake exists. See [PREVIEW.md](PREVIEW.md), [SECURITY.md](SECURITY.md), and [CLA status](CLA.md).

The [distribution contract](DISTRIBUTION.md) provides the same Core-containing `nostos` executable through npm, pinned `npx` zero-install execution, Homebrew, and direct archives. Candidate launchers, packages, archives, and local verification tools are implemented, but none of those channels is published.

The CLI provides one-shot, query-file, piped-stdin, and interactive query modes. Embedded and Source Mode call the public `nostos-engine` facade. Server Mode uses the thin `nostos-client` protocol crate and never opens daemon-managed files or depends on HTTP endpoints. It also provides synchronization, integrity checks, inspection, project diagnostics, graph statistics, and named Database administration.

Build the source-preview executable from this checkout before using the examples:

```bash
cargo build --locked
NOSTOS_BIN="$PWD/target/debug/nostos"
PREVIEW_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nostos-cli.XXXXXX")"
"$NOSTOS_BIN" query 'RETURN 1 AS value' \
  --database "$PREVIEW_DIR/graph.ndb" --format json
```

## Source Mode preview

From the Root multi-repository checkout, this complete disposable workflow uses the installed-provider path and does not require hand-editing `nostos.toml`. The helper writes `.nostos`; every deterministic parse, format, synchronization, query, and `.ndb` operation still goes through the source-built CLI/Core.

```bash
cargo build --manifest-path nostosdb-cli/Cargo.toml --locked
NOSTOS_BIN="$PWD/nostosdb-cli/target/debug/nostos"
PREVIEW_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nostos-source-preview.XXXXXX")"
OWNER="11111111-1111-1111-1111-111111111111"

python3 skills/scripts/nostos_skill.py init \
  --project "$PREVIEW_DIR" \
  --layout centralized \
  --core-provider installed \
  --core-binary "$NOSTOS_BIN" \
  --module-id "$OWNER"

"$NOSTOS_BIN" sync \
  --project "$PREVIEW_DIR" \
  --database "$PREVIEW_DIR/graph.ndb" \
  --format json
"$NOSTOS_BIN" imports --project "$PREVIEW_DIR" --format table
"$NOSTOS_BIN" query \
  "CREATE (n {name: 'Alice'}) RETURN n.name AS name" \
  --project "$PREVIEW_DIR" \
  --database "$PREVIEW_DIR/graph.ndb" \
  --owner "$OWNER" \
  --format json
"$NOSTOS_BIN" query \
  "MATCH (n {name: 'Alice'}) RETURN n.name AS name" \
  --project "$PREVIEW_DIR" \
  --database "$PREVIEW_DIR/graph.ndb" \
  --format json
"$NOSTOS_BIN" doctor \
  --project "$PREVIEW_DIR" \
  --database "$PREVIEW_DIR/graph.ndb" \
  --format json
```

`imports` reports the configured Stable Module IDs. A Source Mode write must use the ID of the one writable owner module; `--owner` without `--project` is rejected before a database can be changed.

`doctor` checks both source compilation/database integrity and the synchronization manifest. It exits nonzero with `source_drift` when current module bytes or semantic identity differ from the materialized database, and with `not_source_managed` when the selected database is unrelated NDB-only state.

For query files or pipes, use the same executable and a database under a disposable or intentional data directory:

```bash
"$NOSTOS_BIN" query --file report.cypher \
  --database "$PREVIEW_DIR/graph.ndb" --format json
cat report.cypher | "$NOSTOS_BIN" query \
  --database "$PREVIEW_DIR/graph.ndb" --format jsonl
"$NOSTOS_BIN" format --file graph.nostos
"$NOSTOS_BIN" format --file graph.nostos --check
```

Use `query --read-only` for tooling that must never execute a graph mutation, including visualization and inspection helpers. For non-interactive one-shot input, the CLI classifies every statement through Core before local synchronization or database creation; it also sets the Server protocol's read-only enforcement flag for remote queries. A permitted Source Mode read can still synchronize the authoritative `.nostos` snapshot before querying it. Interactive Source Mode also synchronizes when the session starts because its later input is not yet available for preflight.

```bash
"$NOSTOS_BIN" query \
  "MATCH (n) RETURN n" \
  --database "$PREVIEW_DIR/graph.ndb" \
  --read-only \
  --format json
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

Output formats are `table`, `json`, `jsonl`, and `csv`; prompts, synchronization warnings, and diagnostics are written to stderr. A single statement in `json` retains the `{columns, rows}` document, while multiple statements produce one JSON array. Multi-statement CSV is rejected because statement schemas may differ; use `jsonl` for streaming multi-statement output. Stable exit codes are 0 (success), 2 (usage), 3 (project/configuration), 4 (query), 5 (database/integrity), 6 (source conflict), and 7 (I/O).

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
python3 distribution/scripts/verify_local.py --skills-root ../skills
```

## License

Source-available under SSPL-1.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
