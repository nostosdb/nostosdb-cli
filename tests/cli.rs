use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "nostdb-cli-{name}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).expect("temporary directory creates");
    path
}

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nostdb"))
}

#[test]
fn one_shot_pipe_and_file_share_machine_readable_semantics() {
    let directory = temp_dir("inputs");
    let database = directory.join("graph.nostdb");

    let output = command()
        .args([
            "query",
            "RETURN 1 AS value",
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "json",
        ])
        .output()
        .expect("one-shot runs");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "{\"columns\":[\"value\"],\"rows\":[[1]]}\n"
    );
    assert!(output.stderr.is_empty());

    let mut child = command()
        .args([
            "query",
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "jsonl",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("pipe process starts");
    child
        .stdin
        .take()
        .expect("stdin exists")
        .write_all(b"RETURN 2 AS value;\n")
        .expect("query writes");
    let output = child.wait_with_output().expect("pipe process exits");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "{\"value\":2}\n"
    );
    assert!(output.stderr.is_empty(), "pipe must not become interactive");

    let query_file = directory.join("queries.cypher");
    fs::write(&query_file, "RETURN 'a,b' AS value;\n").expect("query file writes");
    let output = command()
        .args([
            "query",
            "--file",
            query_file.to_str().expect("UTF-8 path"),
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "csv",
        ])
        .output()
        .expect("file query runs");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "value\n\"a,b\"\n"
    );
    assert!(output.stderr.is_empty());

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn invalid_inputs_and_owner_usage_do_not_create_a_database() {
    let directory = temp_dir("preflight");

    let owner_database = directory.join("owner.nostdb");
    let owner = command()
        .args([
            "query",
            "CREATE (n {name: 'should-not-exist'})",
            "--database",
            owner_database.to_str().expect("UTF-8 path"),
            "--owner",
            "11111111-1111-1111-1111-111111111111",
        ])
        .output()
        .expect("invalid owner invocation runs");
    assert_eq!(owner.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&owner.stderr).contains("--owner requires --project"));
    assert!(!owner_database.exists());

    let read_only_database = directory.join("read-only.nostdb");
    let read_only = command()
        .args([
            "query",
            "CREATE (n {name: 'should-not-exist'})",
            "--database",
            read_only_database.to_str().expect("UTF-8 path"),
            "--read-only",
        ])
        .output()
        .expect("read-only mutation invocation runs");
    assert_eq!(read_only.status.code(), Some(4));
    assert!(read_only.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&read_only.stderr)
            .contains("read-only mode rejects mutating statements")
    );
    assert!(!read_only_database.exists());

    let source_project = directory.join("source-project");
    fs::create_dir(&source_project).expect("source project creates");
    fs::write(
        source_project.join("nostdb.json"),
        "{\"config_version\":2,\"language_version\":1,\"source\":{\"layout\":\"colocated\",\"entry\":\"main.nost\"},\"modules\":{\"main.nost\":\"11111111-1111-1111-1111-111111111111\"}}\n",
    )
    .expect("source configuration writes");
    fs::write(source_project.join("main.nost"), "node existing {}\n").expect("source writes");
    let source_database = source_project.join("graph.nostdb");
    let missing_owner = command()
        .args([
            "query",
            "CREATE (n {name: 'should-not-exist'})",
            "--project",
            source_project.to_str().expect("UTF-8 path"),
            "--database",
            source_database.to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("Source Mode missing-owner invocation runs");
    assert_eq!(missing_owner.status.code(), Some(2));
    assert!(
        !source_database.exists(),
        "owner validation must precede sync"
    );

    let source_read_only = command()
        .args([
            "query",
            "CREATE (n {name: 'should-not-exist'})",
            "--project",
            source_project.to_str().expect("UTF-8 path"),
            "--database",
            source_database.to_str().expect("UTF-8 path"),
            "--owner",
            "11111111-1111-1111-1111-111111111111",
            "--read-only",
        ])
        .output()
        .expect("Source Mode read-only mutation invocation runs");
    assert_eq!(source_read_only.status.code(), Some(4));
    assert!(
        !source_database.exists(),
        "query rejection must precede sync"
    );

    let missing_database = directory.join("missing.nostdb");
    let missing = command()
        .args([
            "query",
            "--file",
            directory
                .join("missing.cypher")
                .to_str()
                .expect("UTF-8 path"),
            "--database",
            missing_database.to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("missing file invocation runs");
    assert_eq!(missing.status.code(), Some(7));
    assert!(!missing_database.exists());

    let invalid_database = directory.join("invalid.nostdb");
    let mut child = command()
        .args([
            "query",
            "--database",
            invalid_database.to_str().expect("UTF-8 path"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("invalid stdin process starts");
    child
        .stdin
        .take()
        .expect("stdin exists")
        .write_all(b"NOT CYPHER\n")
        .expect("invalid query writes");
    let invalid = child.wait_with_output().expect("invalid stdin exits");
    assert_eq!(invalid.status.code(), Some(4));
    assert!(!invalid_database.exists());

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn multi_statement_machine_output_is_framed_or_rejected_before_open() {
    let directory = temp_dir("multi-output");
    let database = directory.join("graph.nostdb");
    let json = command()
        .args([
            "query",
            "RETURN 1 AS first; RETURN 2 AS second;",
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "json",
        ])
        .output()
        .expect("multi-statement JSON runs");
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    let document: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("stdout is one JSON document");
    assert_eq!(document.as_array().expect("batch is an array").len(), 2);
    assert_eq!(document[0]["columns"], serde_json::json!(["first"]));
    assert_eq!(document[1]["rows"], serde_json::json!([[2]]));

    let csv_database = directory.join("csv.nostdb");
    let csv = command()
        .args([
            "query",
            "RETURN 1 AS first; RETURN 2 AS second;",
            "--database",
            csv_database.to_str().expect("UTF-8 path"),
            "--format",
            "csv",
        ])
        .output()
        .expect("multi-statement CSV validation runs");
    assert_eq!(csv.status.code(), Some(2));
    assert!(csv.stdout.is_empty());
    assert!(String::from_utf8_lossy(&csv.stderr).contains("use --format jsonl"));
    assert!(!csv_database.exists());

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn repl_supports_multiline_and_atomic_transactions_without_stdout_prompts() {
    let directory = temp_dir("repl");
    let database = directory.join("graph.nostdb");
    let mut child = command()
        .args([
            "query",
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "jsonl",
            "--interactive",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("REPL starts");
    child
        .stdin
        .take()
        .expect("stdin exists")
        .write_all(
            b":begin\nCREATE (n {name: 'Alice'});\nMATCH (n {name: 'Alice'})\nRETURN n.name AS name;\n:commit\n:quit\n",
        )
        .expect("REPL script writes");
    let output = child.wait_with_output().expect("REPL exits");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 output");
    assert!(stdout.contains("{\"nodes_created\":1"));
    assert!(stdout.contains("{\"name\":\"Alice\"}"));
    assert!(!stdout.contains("nostdb>"));
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 diagnostics");
    assert!(stderr.contains("nostdb>"));
    assert!(stderr.contains("committed"));

    let output = command()
        .args([
            "query",
            "MATCH (n) RETURN n.name AS name",
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "jsonl",
        ])
        .output()
        .expect("verification query runs");
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "{\"name\":\"Alice\"}\n"
    );

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn repl_continues_after_recoverable_meta_command_errors() {
    let directory = temp_dir("repl-recovery");
    let database = directory.join("graph.nostdb");
    let mut child = command()
        .args([
            "query",
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "jsonl",
            "--interactive",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("REPL starts");
    child
        .stdin
        .take()
        .expect("stdin exists")
        .write_all(b":bogus\nRETURN 1 AS after;\n:quit\n")
        .expect("REPL script writes");
    let output = child.wait_with_output().expect("REPL exits");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "{\"after\":1}\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown REPL command"));

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn stable_exit_codes_distinguish_usage_and_query_errors() {
    let usage = command()
        .arg("unknown")
        .output()
        .expect("usage failure runs");
    assert_eq!(usage.status.code(), Some(2));

    let directory = temp_dir("errors");
    let database = directory.join("graph.nostdb");
    let query = command()
        .args([
            "query",
            "NOT CYPHER",
            "--database",
            database.to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("query failure runs");
    assert_eq!(query.status.code(), Some(4));
    assert!(query.stdout.is_empty());
    assert!(String::from_utf8_lossy(&query.stderr).starts_with("nostdb: "));

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn every_subcommand_has_real_help_with_accurate_formats() {
    for subcommand in [
        "query",
        "server",
        "database",
        "sync",
        "format",
        "check",
        "doctor",
        "inspect",
        "stats",
        "schema",
        "unresolved",
        "imports",
        "warnings",
    ] {
        let output = command()
            .args([subcommand, "--help"])
            .output()
            .expect("subcommand help runs");
        assert!(
            output.status.success(),
            "{subcommand}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty(), "{subcommand}");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .starts_with(&format!("Usage: nostdb {subcommand}")),
            "{subcommand}"
        );
    }

    for subcommand in [
        "query",
        "database",
        "sync",
        "check",
        "doctor",
        "inspect",
        "stats",
        "schema",
        "unresolved",
        "imports",
        "warnings",
    ] {
        let output = command()
            .args([subcommand, "--help"])
            .output()
            .expect("format-aware help runs");
        assert!(String::from_utf8_lossy(&output.stdout).contains("table|json|jsonl|csv"));
    }
}

#[test]
fn format_outputs_canonical_source_without_mutating_the_file() {
    let directory = temp_dir("format");
    let source = directory.join("main.nost");
    fs::write(&source, "// retained\nnode alice{name:\"Alice\"}\n").expect("source writes");

    let output = command()
        .args(["format", "--file", source.to_str().expect("UTF-8 path")])
        .output()
        .expect("format runs");
    assert!(output.status.success());
    let canonical = "// retained\nnode alice {\n  name: \"Alice\"\n}\n";
    assert_eq!(String::from_utf8(output.stdout).expect("UTF-8"), canonical);
    assert_eq!(
        fs::read_to_string(&source).expect("original reads"),
        "// retained\nnode alice{name:\"Alice\"}\n"
    );

    let check = command()
        .args([
            "format",
            "--file",
            source.to_str().expect("UTF-8 path"),
            "--check",
        ])
        .output()
        .expect("format check runs");
    assert_eq!(check.status.code(), Some(3));
    assert!(check.stdout.is_empty());
    assert!(String::from_utf8_lossy(&check.stderr).contains("not canonically formatted"));

    fs::write(&source, canonical).expect("canonical source writes");
    assert!(
        command()
            .args([
                "format",
                "--file",
                source.to_str().expect("UTF-8 path"),
                "--check",
            ])
            .status()
            .expect("canonical check runs")
            .success()
    );

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn source_failures_report_file_range_code_severity_and_message() {
    const OWNER: &str = "11111111-1111-1111-1111-111111111111";
    let directory = temp_dir("diagnostics");
    let source = directory.join("main.nost");
    let database = directory.join("graph.nostdb");
    fs::write(
        directory.join("nostdb.json"),
        format!(
            "{{\"config_version\":2,\"language_version\":1,\"source\":{{\"layout\":\"colocated\",\"entry\":\"main.nost\"}},\"modules\":{{\"main.nost\":\"{OWNER}\"}}}}\n"
        ),
    )
    .expect("configuration writes");
    fs::write(&source, "node broken {\n  name: \"unterminated\n}\n")
        .expect("invalid source writes");

    let sync = command()
        .args([
            "sync",
            "--project",
            directory.to_str().expect("UTF-8 path"),
            "--database",
            database.to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("invalid sync runs");
    assert_eq!(sync.status.code(), Some(3));
    assert!(sync.stdout.is_empty());
    let sync_error = String::from_utf8(sync.stderr).expect("UTF-8 diagnostics");
    for expected in [
        "main.nost:bytes ",
        "NOSTDB-L004",
        "error:",
        "closing delimiter",
    ] {
        assert!(
            sync_error.contains(expected),
            "missing {expected:?}: {sync_error}"
        );
    }
    assert!(!database.exists());

    let format = command()
        .args(["format", "--file", source.to_str().expect("UTF-8 path")])
        .output()
        .expect("invalid format runs");
    assert_eq!(format.status.code(), Some(3));
    assert!(format.stdout.is_empty());
    let format_error = String::from_utf8(format.stderr).expect("UTF-8 diagnostics");
    for expected in [
        source.to_str().expect("UTF-8 path"),
        ":bytes ",
        "NOSTDB-L004",
        "error:",
        "closing delimiter",
    ] {
        assert!(
            format_error.contains(expected),
            "missing {expected:?}: {format_error}"
        );
    }

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn automatic_source_sync_reports_warnings_only_on_stderr() {
    const MAIN: &str = "11111111-1111-1111-1111-111111111111";
    const PEOPLE: &str = "22222222-2222-2222-2222-222222222222";
    let directory = temp_dir("sync-warnings");
    let database = directory.join("graph.nostdb");
    fs::write(
        directory.join("nostdb.json"),
        format!(
            "{{\"config_version\":2,\"language_version\":1,\"source\":{{\"layout\":\"colocated\",\"entry\":\"main.nost\"}},\"modules\":{{\"main.nost\":\"{MAIN}\",\"people.nost\":\"{PEOPLE}\"}}}}\n"
        ),
    )
    .expect("configuration writes");
    fs::write(
        directory.join("main.nost"),
        "import \"./people.nost\" as people\nnode alice {}\nedge alice -> people.bob {}\n",
    )
    .expect("source writes");

    let query = command()
        .args([
            "query",
            "RETURN 1 AS value",
            "--project",
            directory.to_str().expect("UTF-8 path"),
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "json",
        ])
        .output()
        .expect("auto-sync query runs");
    assert!(
        query.status.success(),
        "{}",
        String::from_utf8_lossy(&query.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&query.stdout).expect("stdout is JSON"),
        serde_json::json!({"columns": ["value"], "rows": [[1]]})
    );
    let warnings = String::from_utf8(query.stderr).expect("UTF-8 diagnostics");
    assert!(warnings.contains("main.nost:bytes "));
    assert!(warnings.contains("NOSTDB-R005 warning:"));
    assert!(warnings.contains("NOSTDB-R006 warning:"));

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn edge_json_includes_stable_direction_kind_vocabulary() {
    const OWNER: &str = "11111111-1111-1111-1111-111111111111";
    let directory = temp_dir("edge-kind");
    let database = directory.join("graph.nostdb");
    fs::write(
        directory.join("nostdb.json"),
        format!(
            "{{\"config_version\":2,\"language_version\":1,\"source\":{{\"layout\":\"colocated\",\"entry\":\"main.nost\"}},\"modules\":{{\"main.nost\":\"{OWNER}\"}}}}\n"
        ),
    )
    .expect("configuration writes");
    fs::write(
        directory.join("main.nost"),
        "schema DIRECTED {}\nschema DIRECTIONLESS {}\nschema BIDIRECTIONAL {}\n\nnode a {}\nnode b {}\n\nedge a -> b: DIRECTED {}\nedge a - b: DIRECTIONLESS {}\nedge a <-> b: BIDIRECTIONAL {}\n",
    )
    .expect("source writes");
    let sync = command()
        .args([
            "sync",
            "--project",
            directory.to_str().expect("UTF-8 path"),
            "--database",
            database.to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("sync runs");
    assert!(
        sync.status.success(),
        "{}",
        String::from_utf8_lossy(&sync.stderr)
    );

    let output = command()
        .args([
            "query",
            "MATCH (a)-[r]-(b) RETURN r",
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "json",
        ])
        .output()
        .expect("edge query runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("edge output is JSON");
    let kinds = document["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|row| row.get(0))
        .filter_map(|edge| edge.get("kind"))
        .filter_map(serde_json::Value::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        kinds,
        ["bidirectional", "directed", "directionless"]
            .into_iter()
            .collect()
    );

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn doctor_rejects_source_drift_and_unrelated_databases() {
    const OWNER: &str = "11111111-1111-1111-1111-111111111111";
    let directory = temp_dir("doctor-drift");
    let database = directory.join("graph.nostdb");
    fs::write(
        directory.join("nostdb.json"),
        format!(
            "{{\"config_version\":2,\"language_version\":1,\"source\":{{\"layout\":\"colocated\",\"entry\":\"main.nost\"}},\"modules\":{{\"main.nost\":\"{OWNER}\"}}}}\n"
        ),
    )
    .expect("configuration writes");
    fs::write(
        directory.join("main.nost"),
        "node alice {\n  name: \"Alice\"\n  age: 30\n}\n",
    )
    .expect("source writes");
    let sync = command()
        .args([
            "sync",
            "--project",
            directory.to_str().expect("UTF-8 path"),
            "--database",
            database.to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("sync runs");
    assert!(
        sync.status.success(),
        "{}",
        String::from_utf8_lossy(&sync.stderr)
    );

    fs::write(
        directory.join("main.nost"),
        "node alice {\n  name: \"Alice\"\n  age: 31\n}\n",
    )
    .expect("source changes without synchronization");
    let drift = command()
        .args([
            "doctor",
            "--project",
            directory.to_str().expect("UTF-8 path"),
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "json",
        ])
        .output()
        .expect("drift doctor runs");
    assert_eq!(drift.status.code(), Some(3));
    let drift_status: serde_json::Value =
        serde_json::from_slice(&drift.stdout).expect("drift status remains machine-readable");
    assert_eq!(drift_status["rows"][0][2], false);
    assert_eq!(drift_status["rows"][0][3], "source_drift");
    assert!(String::from_utf8_lossy(&drift.stderr).contains("run `nostdb sync`"));

    let unrelated = directory.join("unrelated.nostdb");
    let create = command()
        .args([
            "query",
            "RETURN 1",
            "--database",
            unrelated.to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("unrelated database creates");
    assert!(create.status.success());
    let unrelated_status = command()
        .args([
            "doctor",
            "--project",
            directory.to_str().expect("UTF-8 path"),
            "--database",
            unrelated.to_str().expect("UTF-8 path"),
            "--format",
            "json",
        ])
        .output()
        .expect("unrelated doctor runs");
    assert_eq!(unrelated_status.status.code(), Some(5));
    let status: serde_json::Value = serde_json::from_slice(&unrelated_status.stdout)
        .expect("unrelated status remains machine-readable");
    assert_eq!(status["rows"][0][2], false);
    assert_eq!(status["rows"][0][3], "not_source_managed");

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn source_sync_write_and_administration_use_the_engine_facade() {
    const OWNER: &str = "11111111-1111-1111-1111-111111111111";
    let directory = temp_dir("source");
    let database = directory.join("graph.nostdb");
    fs::write(
        directory.join("nostdb.json"),
        format!(
            "{{\"config_version\":2,\"language_version\":1,\"source\":{{\"layout\":\"colocated\",\"entry\":\"main.nost\"}},\"modules\":{{\"main.nost\":\"{OWNER}\"}}}}\n"
        ),
    )
    .expect("configuration writes");
    fs::write(
        directory.join("main.nost"),
        "schema Person {\n  name: string\n  age: integer\n\n  constraints {\n    required name\n  }\n}\n\nnode alice: Person {\n  name: \"Alice\"\n  age: 30\n}\n",
    )
    .expect("source writes");

    let sync = command()
        .args([
            "sync",
            "--project",
            directory.to_str().expect("UTF-8 path"),
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "json",
        ])
        .output()
        .expect("sync runs");
    assert!(
        sync.status.success(),
        "{}",
        String::from_utf8_lossy(&sync.stderr)
    );
    assert!(String::from_utf8_lossy(&sync.stdout).starts_with("{\"columns\":"));

    for administration in ["check", "inspect", "stats", "schema", "unresolved"] {
        let output = command()
            .args([
                administration,
                "--database",
                database.to_str().expect("UTF-8 path"),
                "--format",
                "json",
            ])
            .output()
            .expect("administration runs");
        assert!(
            output.status.success(),
            "{administration}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\"columns\":"));
        if administration == "schema" {
            assert!(String::from_utf8_lossy(&output.stdout).contains("\"property_type\""));
        }
        if administration == "unresolved" {
            assert!(String::from_utf8_lossy(&output.stdout).contains("\"internal_id\""));
        }
    }
    for administration in ["imports", "warnings"] {
        let output = command()
            .args([
                administration,
                "--project",
                directory.to_str().expect("UTF-8 path"),
                "--format",
                "json",
            ])
            .output()
            .expect("project administration runs");
        assert!(
            output.status.success(),
            "{administration}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("{\"columns\":"));
    }
    let formatted = command()
        .args([
            "format",
            "--file",
            directory.join("main.nost").to_str().expect("UTF-8 path"),
            "--project",
            directory.to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("project-version format runs");
    assert!(
        formatted.status.success(),
        "{}",
        String::from_utf8_lossy(&formatted.stderr)
    );
    assert!(String::from_utf8_lossy(&formatted.stdout).contains("node alice: Person"));
    let doctor = command()
        .args([
            "doctor",
            "--project",
            directory.to_str().expect("UTF-8 path"),
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--format",
            "json",
        ])
        .output()
        .expect("doctor runs");
    assert!(doctor.status.success());

    let write = command()
        .args([
            "query",
            "MATCH (n:Person {name: 'Alice'}) SET n.age = 31 RETURN n.age AS age",
            "--project",
            directory.to_str().expect("UTF-8 path"),
            "--database",
            database.to_str().expect("UTF-8 path"),
            "--owner",
            OWNER,
            "--format",
            "json",
        ])
        .output()
        .expect("source write runs");
    assert!(
        write.status.success(),
        "{}",
        String::from_utf8_lossy(&write.stderr)
    );
    assert_eq!(
        String::from_utf8(write.stdout).expect("UTF-8 output"),
        "{\"columns\":[\"age\"],\"rows\":[[31]]}\n"
    );
    assert!(
        fs::read_to_string(directory.join("main.nost"))
            .expect("source reads")
            .contains("age: 31")
    );

    fs::remove_dir_all(directory).expect("temporary directory removes");
}
