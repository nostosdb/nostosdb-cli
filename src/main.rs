#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nostos_engine::{
    DatabaseError, Digest, EmbeddedDatabase, Parameters, ProjectCompiler, ProjectConfig,
    QueryResult, QueryValue, SourceWriteOptions, SourceWriter, StatementResult, Synchronizer,
    WriteResult, prepare, prepare_write,
};

const EXIT_SUCCESS: u8 = 0;
const EXIT_USAGE: u8 = 2;
const EXIT_PROJECT: u8 = 3;
const EXIT_QUERY: u8 = 4;
const EXIT_DATABASE: u8 = 5;
const EXIT_CONFLICT: u8 = 6;
const EXIT_IO: u8 = 7;

const HELP: &str = "NostosDB command-line client

Usage:
    nostos query [QUERY] [--file PATH] [--database PATH] [--project PATH]
                 [--owner MODULE_ID] [--format table|json|jsonl|csv] [--interactive]
    nostos sync --project PATH --database PATH [--format table|json]
    nostos check|inspect|stats --database PATH [--format table|json]
    nostos doctor --project PATH --database PATH [--format table|json]
    nostos --help
    nostos --version

Exit codes: 0 success, 2 usage, 3 project/configuration, 4 query,
            5 database/integrity, 6 source conflict, 7 I/O.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputFormat {
    Table,
    Json,
    Jsonl,
    Csv,
}

impl OutputFormat {
    fn parse(value: &str) -> Result<Self, CliError> {
        match value {
            "table" => Ok(Self::Table),
            "json" => Ok(Self::Json),
            "jsonl" => Ok(Self::Jsonl),
            "csv" => Ok(Self::Csv),
            _ => Err(CliError::usage(format!("unknown output format `{value}`"))),
        }
    }
}

#[derive(Debug)]
struct CliError {
    code: u8,
    message: String,
}

impl CliError {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn usage(message: impl Into<String>) -> Self {
        Self::new(EXIT_USAGE, message)
    }

    fn project(message: impl Into<String>) -> Self {
        Self::new(EXIT_PROJECT, message)
    }

    fn database(error: DatabaseError) -> Self {
        match error {
            DatabaseError::Query(error) => Self::new(EXIT_QUERY, error.to_string()),
            DatabaseError::Storage(error) => Self::new(EXIT_DATABASE, error.to_string()),
        }
    }
}

#[derive(Debug)]
struct CommonOptions {
    database: PathBuf,
    project: Option<PathBuf>,
    format: OutputFormat,
}

#[derive(Debug)]
struct QueryOptions {
    common: CommonOptions,
    query: Option<String>,
    file: Option<PathBuf>,
    owner: Option<String>,
    interactive: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::from(EXIT_SUCCESS),
        Err(error) => {
            eprintln!("nostos: {}", error.message);
            ExitCode::from(error.code)
        }
    }
}

fn run() -> Result<(), CliError> {
    let mut arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() || matches!(arguments[0].as_str(), "-h" | "--help") {
        println!("{HELP}");
        return Ok(());
    }
    if matches!(arguments[0].as_str(), "-V" | "--version") {
        if arguments.len() != 1 {
            return Err(CliError::usage("--version does not accept arguments"));
        }
        println!("nostos {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let command = arguments.remove(0);
    match command.as_str() {
        "query" => run_query(parse_query(arguments)?),
        "sync" => run_sync(parse_common(arguments, true)?),
        "check" => run_check(parse_common(arguments, false)?),
        "doctor" => run_doctor(parse_common(arguments, true)?),
        "inspect" => run_inspect(parse_common(arguments, false)?),
        "stats" => run_stats(parse_common(arguments, false)?),
        _ => Err(CliError::usage(format!(
            "unknown command `{command}`\n\n{HELP}"
        ))),
    }
}

fn parse_query(arguments: Vec<String>) -> Result<QueryOptions, CliError> {
    let mut database: Option<PathBuf> = None;
    let mut project: Option<PathBuf> = None;
    let mut format = OutputFormat::Table;
    let mut query = None;
    let mut file = None;
    let mut owner = None;
    let mut interactive = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "-d" | "--database" => database = Some(value(&arguments, &mut index)?.into()),
            "-p" | "--project" => project = Some(value(&arguments, &mut index)?.into()),
            "-f" | "--file" => file = Some(value(&arguments, &mut index)?.into()),
            "--owner" => owner = Some(value(&arguments, &mut index)?.to_owned()),
            "--format" => format = OutputFormat::parse(value(&arguments, &mut index)?)?,
            "--interactive" => interactive = true,
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(EXIT_SUCCESS.into());
            }
            option if option.starts_with('-') => {
                return Err(CliError::usage(format!("unknown option `{option}`")));
            }
            text => {
                if query.replace(text.to_owned()).is_some() {
                    return Err(CliError::usage("only one inline query is allowed"));
                }
            }
        }
        index += 1;
    }
    if query.is_some() && file.is_some() {
        return Err(CliError::usage("QUERY and --file are mutually exclusive"));
    }
    if interactive && (query.is_some() || file.is_some()) {
        return Err(CliError::usage(
            "--interactive cannot be combined with QUERY or --file",
        ));
    }
    let database = database.unwrap_or_else(|| {
        project
            .as_ref()
            .map_or_else(|| PathBuf::from("graph.ndb"), |root| root.join("graph.ndb"))
    });
    Ok(QueryOptions {
        common: CommonOptions {
            database,
            project,
            format,
        },
        query,
        file,
        owner,
        interactive,
    })
}

fn parse_common(arguments: Vec<String>, project_required: bool) -> Result<CommonOptions, CliError> {
    let mut database = None;
    let mut project = None;
    let mut format = OutputFormat::Table;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "-d" | "--database" => database = Some(value(&arguments, &mut index)?.into()),
            "-p" | "--project" => project = Some(value(&arguments, &mut index)?.into()),
            "--format" => format = OutputFormat::parse(value(&arguments, &mut index)?)?,
            option => return Err(CliError::usage(format!("unknown option `{option}`"))),
        }
        index += 1;
    }
    if project_required && project.is_none() {
        return Err(CliError::usage("--project is required"));
    }
    let database = database.ok_or_else(|| CliError::usage("--database is required"))?;
    Ok(CommonOptions {
        database,
        project,
        format,
    })
}

fn value<'a>(arguments: &'a [String], index: &mut usize) -> Result<&'a str, CliError> {
    *index += 1;
    arguments
        .get(*index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage("option requires a value"))
}

fn run_query(options: QueryOptions) -> Result<(), CliError> {
    if let Some(project) = &options.common.project {
        synchronize(project, &options.common.database)?;
    }
    let mut database = open_or_create(&options.common.database)?;
    if options.interactive
        || (options.query.is_none() && options.file.is_none() && io::stdin().is_terminal())
    {
        return repl(options, &mut database);
    }
    let source = if let Some(query) = &options.query {
        query.clone()
    } else if let Some(file) = &options.file {
        fs::read_to_string(file).map_err(|error| {
            CliError::new(EXIT_IO, format!("cannot read {}: {error}", file.display()))
        })?
    } else {
        let mut source = String::new();
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| CliError::new(EXIT_IO, format!("cannot read stdin: {error}")))?;
        source
    };
    let statements = split_complete(&source, true)?;
    if statements.is_empty() {
        return Err(CliError::usage("query input is empty"));
    }
    for statement in statements {
        let result = execute_one(&options, &mut database, &statement)?;
        render_statement(&result, options.common.format, &mut io::stdout())?;
    }
    database.checkpoint().map_err(CliError::database)
}

fn execute_one(
    options: &QueryOptions,
    database: &mut EmbeddedDatabase,
    statement: &str,
) -> Result<StatementResult, CliError> {
    let parameters = Parameters::new();
    if prepare(statement).is_ok() {
        return database
            .execute(statement, &parameters)
            .map_err(CliError::database);
    }
    if prepare_write(statement).is_ok() {
        if let Some(project) = &options.common.project {
            let owner_text = options
                .owner
                .as_ref()
                .ok_or_else(|| CliError::usage("Source Mode writes require --owner MODULE_ID"))?;
            let config = ProjectConfig::load(project)
                .map_err(|error| CliError::project(error.to_string()))?;
            let owner_module = owner_text
                .parse()
                .map_err(|error| CliError::usage(format!("invalid --owner: {error}")))?;
            let relative = config
                .modules
                .iter()
                .find_map(|(path, id)| (*id == owner_module).then_some(path))
                .ok_or_else(|| CliError::project("--owner is not mapped by nostos.toml"))?;
            let bytes = fs::read(project.join(relative)).map_err(|error| {
                CliError::new(
                    EXIT_IO,
                    format!("cannot read {}: {error}", relative.display()),
                )
            })?;
            let mut writer = SourceWriter::default();
            return writer
                .execute(
                    project,
                    &options.common.database,
                    statement,
                    &parameters,
                    SourceWriteOptions {
                        owner_module,
                        expected_hash: Digest::of_bytes(&bytes),
                    },
                )
                .map(|report| StatementResult::Write(report.write))
                .map_err(map_source_write);
        }
    }
    database
        .execute(statement, &parameters)
        .map_err(CliError::database)
}

fn map_source_write(error: nostos_engine::SourceWriteError) -> CliError {
    use nostos_engine::SourceWriteError;
    let code = match &error {
        SourceWriteError::Conflict { .. } => EXIT_CONFLICT,
        SourceWriteError::Config(_) | SourceWriteError::Compile(_) => EXIT_PROJECT,
        SourceWriteError::Query(_) => EXIT_QUERY,
        SourceWriteError::Storage(_) | SourceWriteError::StaleDatabase => EXIT_DATABASE,
        SourceWriteError::Io(_) | SourceWriteError::SyncAfterSourceChange(_) => EXIT_IO,
        SourceWriteError::ReadOnlyModule(_)
        | SourceWriteError::UnknownOwner(_)
        | SourceWriteError::Unsupported(_)
        | SourceWriteError::Format(_) => EXIT_QUERY,
    };
    CliError::new(code, error.to_string())
}

fn open_or_create(path: &Path) -> Result<EmbeddedDatabase, CliError> {
    if path.exists() {
        EmbeddedDatabase::open(path).map_err(CliError::database)
    } else {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                CliError::new(
                    EXIT_IO,
                    format!("cannot create {}: {error}", parent.display()),
                )
            })?;
        }
        EmbeddedDatabase::create(path).map_err(CliError::database)
    }
}

fn synchronize(project: &Path, database: &Path) -> Result<nostos_engine::SyncReport, CliError> {
    Synchronizer::default()
        .sync(project, database)
        .map_err(|error| CliError::project(error.to_string()))
}

fn run_sync(options: CommonOptions) -> Result<(), CliError> {
    let report = synchronize(
        options.project.as_deref().expect("required by parser"),
        &options.database,
    )?;
    let rows = vec![vec![
        QueryValue::String(report.semantic_hash.to_string()),
        QueryValue::Integer(i64::from(report.attempts)),
        QueryValue::Integer(report.diagnostics.len() as i64),
    ]];
    render_table_data(
        &["semantic_hash", "attempts", "warnings"],
        &rows,
        options.format,
        &mut io::stdout(),
    )
}

fn run_check(options: CommonOptions) -> Result<(), CliError> {
    let database = EmbeddedDatabase::open(&options.database).map_err(CliError::database)?;
    let status = database.check().map_err(CliError::database)?;
    if status.is_valid() {
        render_table_data(
            &["valid", "findings"],
            &[vec![QueryValue::Boolean(true), QueryValue::Integer(0)]],
            options.format,
            &mut io::stdout(),
        )
    } else {
        for finding in &status.findings {
            eprintln!("{}: {}", finding.kind, finding.message);
        }
        Err(CliError::new(
            EXIT_DATABASE,
            format!("integrity check found {} problem(s)", status.findings.len()),
        ))
    }
}

fn run_inspect(options: CommonOptions) -> Result<(), CliError> {
    let database = EmbeddedDatabase::open(&options.database).map_err(CliError::database)?;
    let info = database.info().map_err(CliError::database)?;
    render_table_data(
        &[
            "ndb_format",
            "schema_revision",
            "generation",
            "logical_checksum",
            "source_managed",
        ],
        &[vec![
            QueryValue::Integer(i64::from(info.ndb_format_version)),
            QueryValue::Integer(i64::from(info.schema_revision)),
            QueryValue::Integer(info.generation as i64),
            QueryValue::String(format!("{:016x}", info.logical_checksum)),
            QueryValue::Boolean(info.source_managed),
        ]],
        options.format,
        &mut io::stdout(),
    )
}

fn run_stats(options: CommonOptions) -> Result<(), CliError> {
    let database = EmbeddedDatabase::open(&options.database).map_err(CliError::database)?;
    let counts = database.counts().map_err(CliError::database)?;
    render_table_data(
        &["schemas", "nodes", "edges", "adjacency", "properties"],
        &[vec![
            QueryValue::Integer(counts.schemas as i64),
            QueryValue::Integer(counts.nodes as i64),
            QueryValue::Integer(counts.edges as i64),
            QueryValue::Integer(counts.adjacency as i64),
            QueryValue::Integer(counts.properties as i64),
        ]],
        options.format,
        &mut io::stdout(),
    )
}

fn run_doctor(options: CommonOptions) -> Result<(), CliError> {
    let project = options.project.as_deref().expect("required by parser");
    let mut compiler = ProjectCompiler::new();
    let compiled = compiler
        .compile(project)
        .map_err(|error| CliError::project(error.to_string()))?;
    let database = EmbeddedDatabase::open(&options.database).map_err(CliError::database)?;
    let status = database.check().map_err(CliError::database)?;
    if !status.is_valid() {
        return Err(CliError::new(
            EXIT_DATABASE,
            "database integrity check failed",
        ));
    }
    render_table_data(
        &["project", "database", "warnings"],
        &[vec![
            QueryValue::String("ok".to_owned()),
            QueryValue::String("ok".to_owned()),
            QueryValue::Integer(compiled.diagnostics.len() as i64),
        ]],
        options.format,
        &mut io::stdout(),
    )
}

fn repl(options: QueryOptions, database: &mut EmbeddedDatabase) -> Result<(), CliError> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let mut line = String::new();
    let mut buffer = String::new();
    let mut transaction: Option<Vec<(String, Parameters)>> = None;
    loop {
        write!(
            stderr,
            "{}",
            if buffer.is_empty() {
                "nostos> "
            } else {
                "...> "
            }
        )
        .map_err(output_error)?;
        stderr.flush().map_err(output_error)?;
        line.clear();
        if input.read_line(&mut line).map_err(|error| {
            CliError::new(EXIT_IO, format!("cannot read interactive input: {error}"))
        })? == 0
        {
            if !buffer.trim().is_empty() {
                return Err(CliError::usage("unterminated query at end of input"));
            }
            break;
        }
        let trimmed = line.trim();
        if buffer.is_empty() && trimmed.starts_with(':') {
            if handle_admin(
                trimmed,
                &options,
                database,
                &mut transaction,
                &mut stdout,
                &mut stderr,
            )? {
                break;
            }
            continue;
        }
        buffer.push_str(&line);
        let (statements, remainder) = split_with_remainder(&buffer)?;
        buffer = remainder;
        if buffer.trim().is_empty() {
            buffer.clear();
        }
        for statement in statements {
            if let Some(pending) = &mut transaction {
                pending.push((statement, Parameters::new()));
                writeln!(stderr, "queued").map_err(output_error)?;
            } else {
                match execute_one(&options, database, &statement) {
                    Ok(result) => render_statement(&result, options.common.format, &mut stdout)?,
                    Err(error) => {
                        writeln!(stderr, "error: {}", error.message).map_err(output_error)?
                    }
                }
            }
        }
    }
    Ok(())
}

fn handle_admin(
    command: &str,
    options: &QueryOptions,
    database: &mut EmbeddedDatabase,
    transaction: &mut Option<Vec<(String, Parameters)>>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<bool, CliError> {
    match command {
        ":quit" | ":q" => return Ok(true),
        ":help" => writeln!(
            stdout,
            ":help :status :sync :schema :warnings :imports :unresolved\n:begin :commit :rollback :quit"
        )
        .map_err(output_error)?,
        ":status" => {
            let info = database.info().map_err(CliError::database)?;
            render_table_data(
                &["generation", "source_managed"],
                &[vec![
                    QueryValue::Integer(info.generation as i64),
                    QueryValue::Boolean(info.source_managed),
                ]],
                options.common.format,
                stdout,
            )?;
        }
        ":sync" => {
            let project = options
                .common
                .project
                .as_deref()
                .ok_or_else(|| CliError::usage(":sync requires --project"))?;
            synchronize(project, &options.common.database)?;
            *database = EmbeddedDatabase::open(&options.common.database).map_err(CliError::database)?;
            writeln!(stderr, "synchronized").map_err(output_error)?;
        }
        ":schema" => {
            let schemas = database.schemas().map_err(CliError::database)?;
            let rows = schemas
                .into_iter()
                .map(|schema| {
                    vec![
                        QueryValue::String(schema.identity),
                        QueryValue::String(schema.state),
                        QueryValue::Integer(schema.properties.len() as i64),
                        QueryValue::Integer(schema.constraints as i64),
                    ]
                })
                .collect::<Vec<_>>();
            render_table_data(
                &["identity", "state", "properties", "constraints"],
                &rows,
                options.common.format,
                stdout,
            )?;
        }
        ":unresolved" => {
            let values = database.unresolved().map_err(CliError::database)?;
            let rows = values
                .into_iter()
                .map(|value| {
                    vec![
                        QueryValue::String(value.kind),
                        QueryValue::String(value.identity),
                        QueryValue::String(value.state),
                    ]
                })
                .collect::<Vec<_>>();
            render_table_data(
                &["kind", "identity", "state"],
                &rows,
                options.common.format,
                stdout,
            )?;
        }
        ":warnings" | ":imports" => render_project_admin(command, options, stdout)?,
        ":begin" => {
            if options.common.project.is_some() {
                return Err(CliError::usage(
                    "explicit transactions are available only in NDB-only mode",
                ));
            }
            if transaction.is_some() {
                return Err(CliError::usage("a transaction is already active"));
            }
            *transaction = Some(Vec::new());
            writeln!(stderr, "transaction started").map_err(output_error)?;
        }
        ":commit" => {
            let statements = transaction
                .take()
                .ok_or_else(|| CliError::usage("no transaction is active"))?;
            match database.execute_transaction(&statements) {
                Ok(results) => {
                    for result in results {
                        render_statement(&result, options.common.format, stdout)?;
                    }
                    database.checkpoint().map_err(CliError::database)?;
                    writeln!(stderr, "committed").map_err(output_error)?;
                }
                Err(error) => {
                    writeln!(stderr, "rolled back: {error}").map_err(output_error)?;
                }
            }
        }
        ":rollback" => {
            transaction
                .take()
                .ok_or_else(|| CliError::usage("no transaction is active"))?;
            writeln!(stderr, "rolled back").map_err(output_error)?;
        }
        _ => return Err(CliError::usage(format!("unknown REPL command `{command}`"))),
    }
    Ok(false)
}

fn render_project_admin(
    command: &str,
    options: &QueryOptions,
    output: &mut impl Write,
) -> Result<(), CliError> {
    let project = options
        .common
        .project
        .as_deref()
        .ok_or_else(|| CliError::usage(format!("{command} requires --project")))?;
    if command == ":imports" {
        let config =
            ProjectConfig::load(project).map_err(|error| CliError::project(error.to_string()))?;
        let rows = config
            .modules
            .into_iter()
            .map(|(path, id)| {
                vec![
                    QueryValue::String(path.display().to_string()),
                    QueryValue::String(id.to_string()),
                ]
            })
            .collect::<Vec<_>>();
        return render_table_data(
            &["module", "stable_id"],
            &rows,
            options.common.format,
            output,
        );
    }
    let mut compiler = ProjectCompiler::new();
    let compiled = compiler
        .compile(project)
        .map_err(|error| CliError::project(error.to_string()))?;
    let rows = compiled
        .diagnostics
        .into_iter()
        .map(|diagnostic| {
            vec![
                QueryValue::String(
                    diagnostic
                        .module
                        .map_or_else(|| "<project>".to_owned(), |path| path.display().to_string()),
                ),
                QueryValue::String(diagnostic.diagnostic.code().to_string()),
                QueryValue::String(diagnostic.diagnostic.message().to_owned()),
            ]
        })
        .collect::<Vec<_>>();
    render_table_data(
        &["module", "code", "message"],
        &rows,
        options.common.format,
        output,
    )
}

fn render_statement(
    result: &StatementResult,
    format: OutputFormat,
    output: &mut impl Write,
) -> Result<(), CliError> {
    match result {
        StatementResult::Read(result) => render_query(result, format, output),
        StatementResult::Write(result) => render_write(result, format, output),
    }
}

fn render_query(
    result: &QueryResult,
    format: OutputFormat,
    output: &mut impl Write,
) -> Result<(), CliError> {
    let columns = result
        .columns
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    render_table_data(&columns, &result.rows, format, output)
}

fn render_write(
    result: &WriteResult,
    format: OutputFormat,
    output: &mut impl Write,
) -> Result<(), CliError> {
    if let Some(table) = &result.result {
        render_query(table, format, output)?;
        if format != OutputFormat::Table {
            return Ok(());
        }
    }
    let summary = result.summary;
    let rows = [vec![
        QueryValue::Integer(summary.nodes_created as i64),
        QueryValue::Integer(summary.edges_created as i64),
        QueryValue::Integer(summary.nodes_deleted as i64),
        QueryValue::Integer(summary.edges_deleted as i64),
        QueryValue::Integer(summary.properties_set as i64),
        QueryValue::Integer(summary.properties_removed as i64),
    ]];
    render_table_data(
        &[
            "nodes_created",
            "edges_created",
            "nodes_deleted",
            "edges_deleted",
            "properties_set",
            "properties_removed",
        ],
        &rows,
        format,
        output,
    )
}

fn render_table_data(
    columns: &[&str],
    rows: &[Vec<QueryValue>],
    format: OutputFormat,
    output: &mut impl Write,
) -> Result<(), CliError> {
    match format {
        OutputFormat::Table => render_text_table(columns, rows, output),
        OutputFormat::Json => {
            write!(output, "{{\"columns\":[").map_err(output_error)?;
            for (index, column) in columns.iter().enumerate() {
                if index > 0 {
                    write!(output, ",").map_err(output_error)?;
                }
                write_json_string(output, column)?;
            }
            write!(output, "],\"rows\":[").map_err(output_error)?;
            for (row_index, row) in rows.iter().enumerate() {
                if row_index > 0 {
                    write!(output, ",").map_err(output_error)?;
                }
                write!(output, "[").map_err(output_error)?;
                for (index, value) in row.iter().enumerate() {
                    if index > 0 {
                        write!(output, ",").map_err(output_error)?;
                    }
                    write_json_value(output, value)?;
                }
                write!(output, "]").map_err(output_error)?;
            }
            writeln!(output, "]}}").map_err(output_error)
        }
        OutputFormat::Jsonl => {
            for row in rows {
                write!(output, "{{").map_err(output_error)?;
                for (index, column) in columns.iter().enumerate() {
                    if index > 0 {
                        write!(output, ",").map_err(output_error)?;
                    }
                    write_json_string(output, column)?;
                    write!(output, ":").map_err(output_error)?;
                    write_json_value(output, row.get(index).unwrap_or(&QueryValue::Null))?;
                }
                writeln!(output, "}}").map_err(output_error)?;
            }
            Ok(())
        }
        OutputFormat::Csv => {
            write_csv_row(output, columns.iter().copied())?;
            for row in rows {
                let values = row.iter().map(display_value).collect::<Vec<_>>();
                write_csv_row(output, values.iter().map(String::as_str))?;
            }
            Ok(())
        }
    }
}

fn render_text_table(
    columns: &[&str],
    rows: &[Vec<QueryValue>],
    output: &mut impl Write,
) -> Result<(), CliError> {
    let values = rows
        .iter()
        .map(|row| row.iter().map(display_value).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let widths = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            values
                .iter()
                .filter_map(|row| row.get(index))
                .map(String::len)
                .fold(column.len(), usize::max)
        })
        .collect::<Vec<_>>();
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            write!(output, " | ").map_err(output_error)?;
        }
        write!(output, "{column:<width$}", width = widths[index]).map_err(output_error)?;
    }
    writeln!(output).map_err(output_error)?;
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            write!(output, "-+-").map_err(output_error)?;
        }
        write!(output, "{}", "-".repeat(*width)).map_err(output_error)?;
    }
    writeln!(output).map_err(output_error)?;
    for row in values {
        for (index, value) in row.iter().enumerate() {
            if index > 0 {
                write!(output, " | ").map_err(output_error)?;
            }
            write!(output, "{value:<width$}", width = widths[index]).map_err(output_error)?;
        }
        writeln!(output).map_err(output_error)?;
    }
    Ok(())
}

fn display_value(value: &QueryValue) -> String {
    match value {
        QueryValue::Null => "null".to_owned(),
        QueryValue::Boolean(value) => value.to_string(),
        QueryValue::Integer(value) => value.to_string(),
        QueryValue::Float(value) => value.to_string(),
        QueryValue::String(value) => value.clone(),
        QueryValue::Bytes(value) => value.iter().map(|byte| format!("{byte:02x}")).collect(),
        QueryValue::Duration(value) => format!("{value}ns"),
        QueryValue::List(values) => format!(
            "[{}]",
            values
                .iter()
                .map(display_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        QueryValue::Map(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!("{key}: {}", display_value(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        QueryValue::Node(node) => format!("node#{}", node.id.get()),
        QueryValue::Edge(edge) => format!("edge#{}", edge.id.get()),
    }
}

fn write_json_value(output: &mut impl Write, value: &QueryValue) -> Result<(), CliError> {
    match value {
        QueryValue::Null => write!(output, "null").map_err(output_error),
        QueryValue::Boolean(value) => write!(output, "{value}").map_err(output_error),
        QueryValue::Integer(value) => write!(output, "{value}").map_err(output_error),
        QueryValue::Float(value) => write!(output, "{value}").map_err(output_error),
        QueryValue::String(value) => write_json_string(output, value),
        QueryValue::Bytes(value) => {
            let text = value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            write_json_string(output, &text)
        }
        QueryValue::Duration(value) => write!(output, "{value}").map_err(output_error),
        QueryValue::List(values) => {
            write!(output, "[").map_err(output_error)?;
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    write!(output, ",").map_err(output_error)?;
                }
                write_json_value(output, value)?;
            }
            write!(output, "]").map_err(output_error)
        }
        QueryValue::Map(values) => {
            write!(output, "{{").map_err(output_error)?;
            for (index, (key, value)) in values.iter().enumerate() {
                if index > 0 {
                    write!(output, ",").map_err(output_error)?;
                }
                write_json_string(output, key)?;
                write!(output, ":").map_err(output_error)?;
                write_json_value(output, value)?;
            }
            write!(output, "}}").map_err(output_error)
        }
        QueryValue::Node(node) => {
            write!(output, "{{\"id\":{},\"labels\":[", node.id.get()).map_err(output_error)?;
            for (index, label) in node.labels.iter().enumerate() {
                if index > 0 {
                    write!(output, ",").map_err(output_error)?;
                }
                write_json_string(output, label)?;
            }
            write!(output, "],\"properties\":").map_err(output_error)?;
            write_json_value(output, &QueryValue::Map(node.properties.clone()))?;
            write!(output, "}}").map_err(output_error)
        }
        QueryValue::Edge(edge) => {
            write!(
                output,
                "{{\"id\":{},\"source\":{},\"target\":{},\"type\":",
                edge.id.get(),
                edge.source.get(),
                edge.target.get()
            )
            .map_err(output_error)?;
            match &edge.relationship_type {
                Some(value) => write_json_string(output, value)?,
                None => write!(output, "null").map_err(output_error)?,
            }
            write!(output, ",\"properties\":").map_err(output_error)?;
            write_json_value(output, &QueryValue::Map(edge.properties.clone()))?;
            write!(output, "}}").map_err(output_error)
        }
    }
}

fn write_json_string(output: &mut impl Write, value: &str) -> Result<(), CliError> {
    write!(output, "\"").map_err(output_error)?;
    for character in value.chars() {
        match character {
            '\"' => write!(output, "\\\"").map_err(output_error)?,
            '\\' => write!(output, "\\\\").map_err(output_error)?,
            '\n' => write!(output, "\\n").map_err(output_error)?,
            '\r' => write!(output, "\\r").map_err(output_error)?,
            '\t' => write!(output, "\\t").map_err(output_error)?,
            value if value < ' ' => {
                write!(output, "\\u{:04x}", value as u32).map_err(output_error)?
            }
            value => write!(output, "{value}").map_err(output_error)?,
        }
    }
    write!(output, "\"").map_err(output_error)
}

fn write_csv_row<'a>(
    output: &mut impl Write,
    values: impl Iterator<Item = &'a str>,
) -> Result<(), CliError> {
    for (index, value) in values.enumerate() {
        if index > 0 {
            write!(output, ",").map_err(output_error)?;
        }
        if value.contains([',', '\"', '\n', '\r']) {
            write!(output, "\"{}\"", value.replace('\"', "\"\"")).map_err(output_error)?;
        } else {
            write!(output, "{value}").map_err(output_error)?;
        }
    }
    writeln!(output).map_err(output_error)
}

fn output_error(error: io::Error) -> CliError {
    CliError::new(EXIT_IO, format!("cannot write output: {error}"))
}

fn split_complete(source: &str, accept_remainder: bool) -> Result<Vec<String>, CliError> {
    let (mut statements, remainder) = split_with_remainder(source)?;
    if !remainder.trim().is_empty() {
        if accept_remainder {
            statements.push(remainder.trim().to_owned());
        } else {
            return Err(CliError::usage("unterminated query"));
        }
    }
    Ok(statements)
}

fn split_with_remainder(source: &str) -> Result<(Vec<String>, String), CliError> {
    let bytes = source.as_bytes();
    let mut statements = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut quote = None;
    let mut line_comment = false;
    let mut block_depth = 0_u32;
    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        if line_comment {
            if byte == b'\n' {
                line_comment = false;
            }
        } else if block_depth > 0 {
            if byte == b'/' && next == Some(b'*') {
                block_depth += 1;
                index += 1;
            } else if byte == b'*' && next == Some(b'/') {
                block_depth -= 1;
                index += 1;
            }
        } else if let Some(delimiter) = quote {
            if byte == b'\\' {
                index += usize::from(next.is_some());
            } else if byte == delimiter {
                if next == Some(delimiter) && delimiter == b'`' {
                    index += 1;
                } else {
                    quote = None;
                }
            }
        } else if matches!(byte, b'\'' | b'\"' | b'`') {
            quote = Some(byte);
        } else if byte == b'/' && next == Some(b'/') {
            line_comment = true;
            index += 1;
        } else if byte == b'/' && next == Some(b'*') {
            block_depth = 1;
            index += 1;
        } else if byte == b';' {
            let statement = source[start..index].trim();
            if !statement.is_empty() {
                statements.push(statement.to_owned());
            }
            start = index + 1;
        }
        index += 1;
    }
    if block_depth > 0 || quote.is_some() {
        return Ok((statements, source[start..].to_owned()));
    }
    Ok((statements, source[start..].to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statement_splitter_ignores_semicolons_in_literals_and_comments() {
        let source = "RETURN 'a;b'; // ;\nRETURN `x;y`; /* ; */ RETURN 3";
        assert_eq!(
            split_complete(source, true).expect("splits"),
            ["RETURN 'a;b'", "// ;\nRETURN `x;y`", "/* ; */ RETURN 3"]
        );
    }

    #[test]
    fn json_and_csv_escape_machine_values() {
        let rows = vec![vec![QueryValue::String("a,\"b\n".to_owned())]];
        let mut json = Vec::new();
        render_table_data(&["value"], &rows, OutputFormat::Json, &mut json).expect("JSON");
        assert_eq!(
            String::from_utf8(json).expect("UTF-8"),
            "{\"columns\":[\"value\"],\"rows\":[[\"a,\\\"b\\n\"]]}\n"
        );
        let mut csv = Vec::new();
        render_table_data(&["value"], &rows, OutputFormat::Csv, &mut csv).expect("CSV");
        assert_eq!(
            String::from_utf8(csv).expect("UTF-8"),
            "value\n\"a,\"\"b\n\"\n"
        );
    }
}
