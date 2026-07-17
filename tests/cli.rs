use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "nostos-cli-{name}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).expect("temporary directory creates");
    path
}

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nostos"))
}

#[test]
fn one_shot_pipe_and_file_share_machine_readable_semantics() {
    let directory = temp_dir("inputs");
    let database = directory.join("graph.ndb");

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
fn repl_supports_multiline_and_atomic_transactions_without_stdout_prompts() {
    let directory = temp_dir("repl");
    let database = directory.join("graph.ndb");
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
    assert!(!stdout.contains("nostos>"));
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 diagnostics");
    assert!(stderr.contains("nostos>"));
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
fn stable_exit_codes_distinguish_usage_and_query_errors() {
    let usage = command()
        .arg("unknown")
        .output()
        .expect("usage failure runs");
    assert_eq!(usage.status.code(), Some(2));

    let directory = temp_dir("errors");
    let database = directory.join("graph.ndb");
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
    assert!(String::from_utf8_lossy(&query.stderr).starts_with("nostos: "));

    fs::remove_dir_all(directory).expect("temporary directory removes");
}

#[test]
fn format_outputs_canonical_source_without_mutating_the_file() {
    let directory = temp_dir("format");
    let source = directory.join("main.nostos");
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
fn source_sync_write_and_administration_use_the_engine_facade() {
    const OWNER: &str = "11111111-1111-1111-1111-111111111111";
    let directory = temp_dir("source");
    let database = directory.join("graph.ndb");
    fs::write(
        directory.join("nostos.toml"),
        format!(
            "config_version = 1\nlanguage_version = 1\n\n[source]\nlayout = \"colocated\"\nentry = \"main.nostos\"\n\n[modules]\n\"main.nostos\" = \"{OWNER}\"\n"
        ),
    )
    .expect("configuration writes");
    fs::write(
        directory.join("main.nostos"),
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
            directory.join("main.nostos").to_str().expect("UTF-8 path"),
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
        fs::read_to_string(directory.join("main.nostos"))
            .expect("source reads")
            .contains("age: 31")
    );

    fs::remove_dir_all(directory).expect("temporary directory removes");
}
