# nostdb-cli

Source-available command-line consumer of the NostDB engine, licensed under SSPL-1.0.

Unsupported public-preview source and npm binaries are available, but no
production support, stable database format, or external contribution intake
exists. See [PREVIEW.md](PREVIEW.md), [SECURITY.md](SECURITY.md), and
[CLA status](CLA.md).

The [distribution contract](DISTRIBUTION.md) provides the same Core-containing
`nostdb` executable through npm, pinned `npx` zero-install execution, Homebrew,
and direct archives. The unsupported npm `0.0.1` launcher and six native
packages are published under `latest` and `next`; Homebrew and direct archives
remain unpublished candidates.

```bash
npm install --global @nostdb/cli@0.0.1
nostdb --version
```

The CLI provides one-shot, query-file, piped-stdin, and interactive query
modes. Embedded and human-readable local projects call the public
`nostdb-engine` facade. Server Mode uses the thin `nostdb-client` protocol crate
and never opens daemon-managed files or depends on HTTP endpoints. It also
provides synchronization, integrity checks, inspection, project diagnostics,
graph statistics, and named Database administration.

Build the source-preview executable from this checkout before using the examples:

```bash
cargo build --locked
NOSTDB_BIN="$PWD/target/debug/nostdb"
PREVIEW_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nostdb-cli.XXXXXX")"
"$NOSTDB_BIN" query 'RETURN 1 AS value' \
  --database "$PREVIEW_DIR/.nostdb" --format json
```

## NDB-first project preview

From the Root multi-repository checkout, this workflow initializes only
`.nostdb`. Source becomes visible only after `nost` is enabled; every
database write, source materialization, parse, format, and synchronization still
goes through the CLI/Core.

```bash
cargo build --manifest-path nostdb-cli/Cargo.toml --locked
NOSTDB_BIN="$PWD/nostdb-cli/target/debug/nostdb"
PREVIEW_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nostdb-source-preview.XXXXXX")"

python3 skills/scripts/nostdb_skill.py init \
  --src "$PREVIEW_DIR" \
  --core-provider installed \
  --core-binary "$NOSTDB_BIN"
"$NOSTDB_BIN" query \
  "CREATE (n {name: 'Alice'}) RETURN n.name AS name" \
  --project "$PREVIEW_DIR" \
  --format json
"$NOSTDB_BIN" query \
  "MATCH (n {name: 'Alice'}) RETURN n.name AS name" \
  --project "$PREVIEW_DIR" \
  --format json
"$NOSTDB_BIN" doctor \
  --project "$PREVIEW_DIR" \
  --format json

python3 skills/scripts/nostdb_project.py configure \
  --src "$PREVIEW_DIR" --nost true
"$NOSTDB_BIN" sync --project "$PREVIEW_DIR" --format json
"$NOSTDB_BIN" imports --project "$PREVIEW_DIR" --format table
```

Project queries use top-level `root: ".nostdb"` from `nostdb.json`. Writes
commit to `.nostdb` first. When `nost: true`, post-write synchronization updates
canonical human-readable `.nost`; direct source edits are applied back when
their content changed, and the later update time wins when both representations
changed.

`doctor` reports `ndb_only` for the default hidden-source project. For projects
with `nost: true` it checks source compilation, database integrity, database
generation, and module hashes, and reports `source_drift` when they differ.

For query files or pipes, use the same executable and a database under a disposable or intentional data directory:

```bash
"$NOSTDB_BIN" query --file report.cypher \
  --database "$PREVIEW_DIR/.nostdb" --format json
cat report.cypher | "$NOSTDB_BIN" query \
  --database "$PREVIEW_DIR/.nostdb" --format jsonl
"$NOSTDB_BIN" format --file graph.nost
"$NOSTDB_BIN" format --file graph.nost --check
```

Use `query --read-only` for tooling that must never execute a graph mutation,
including visualization and inspection helpers. For non-interactive one-shot
input, the CLI classifies every statement through Core before local
synchronization or database creation; it also sets the Server protocol's
read-only enforcement flag for remote queries. Project queries reconcile
optional human-readable source before opening `.nostdb`, and mutating queries
reconcile it again after the database commit.

```bash
"$NOSTDB_BIN" query \
  "MATCH (n) RETURN n" \
  --database "$PREVIEW_DIR/.nostdb" \
  --read-only \
  --format json
```

With an explicitly installed `nostd`:

```bash
nostdb server ping --server nostdb://127.0.0.1:7878 \
  --credential-file /var/lib/nostdb/credentials/client.token
nostdb database create knowledge --server nostdb://127.0.0.1:7878 \
  --credential-file /var/lib/nostdb/credentials/admin.token
nostdb query 'MATCH (n) RETURN n' --server nostdb://127.0.0.1:7878 \
  --database knowledge \
  --credential-file /var/lib/nostdb/credentials/client.token
```

Database commands include create/list/inspect/rename, exact-name-confirmed drop, physical snapshot/restore, and logical export/import. The remote REPL supports `:ping`, `:begin`, `:commit`, and `:rollback`. Credentials come only from `NOSTDB_CREDENTIAL` or `--credential-file`; there is no credential command-line value.

`format` sends one complete source file through the public Core canonical formatter. By default it writes the formatted source to stdout without changing the input; `--check` returns project exit code 3 when the input is not already canonical. Use `--project` to read `nostdb` from `nostdb.json`, or supply `--nostdb-version` explicitly.

Standalone `schema`, `unresolved`, `imports`, and `warnings` commands expose the same administration data as the REPL in table or JSON form for scripts and Agent Skills.

Interactive statements end with `;` and may span lines. Administrative commands
include `:status`, `:sync`, `:schema`, `:warnings`, `:imports`, `:unresolved`,
and explicit transactions. Local project queries use `--project` and always
open the root `.nostdb`.

Output formats are `table`, `json`, `jsonl`, and `csv`; prompts, synchronization warnings, and diagnostics are written to stderr. A single statement in `json` retains the `{columns, rows}` document, while multiple statements produce one JSON array. Multi-statement CSV is rejected because statement schemas may differ; use `jsonl` for streaming multi-statement output. Stable exit codes are 0 (success), 2 (usage), 3 (project/configuration), 4 (query), 5 (database/integrity), 6 (source conflict), and 7 (I/O).

The local development manifest depends on sibling `nostdb-engine` and the sibling Server's Core-free `nostdb-client` crate by path and compatible version. It does not access Core storage internals or Server daemon internals.

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
