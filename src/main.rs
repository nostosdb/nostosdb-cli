#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use nostdb_client::{
    Client, ClientError, ClientRequest, ErrorCode as RemoteErrorCode, SNAPSHOT_CHUNK_BYTES,
    ServerResponse,
};
use nostdb_engine::{
    CompileError, DEFAULT_DATABASE_FILE, DatabaseError, EdgeKind, EmbeddedDatabase,
    NOST_LANGUAGE_VERSION, PROJECT_DIRECTORY, Parameters, ProjectCompiler, ProjectConfig,
    ProjectDiagnostic, ProjectSyncDirection, ProjectSyncError, ProjectSyncReport,
    ProjectSynchronizer, QueryResult, QueryValue, SETTINGS_FILE, SETTINGS_VERSION, SchemaInfo,
    StatementResult, UnresolvedInfo, WriteResult, format_source, prepare, prepare_write,
};

const EXIT_SUCCESS: u8 = 0;
const EXIT_USAGE: u8 = 2;
const EXIT_PROJECT: u8 = 3;
const EXIT_QUERY: u8 = 4;
const EXIT_DATABASE: u8 = 5;
const EXIT_CONFLICT: u8 = 6;
const EXIT_IO: u8 = 7;
const MACHINE_FORMATS: &str = "table|json|jsonl|csv";
static FORMAT_DIAGNOSTIC_SEQUENCE: AtomicU64 = AtomicU64::new(1);

const HELP: &str = "NostDB command-line client

Usage:
    nostdb init --project PATH [--database FILE] [--allow-nonempty]
                [--format table|json|jsonl|csv]
    nostdb update --project PATH [--format table|json|jsonl|csv]
    nostdb query [QUERY] [--file PATH] [--database PATH|NAME] [--project PATH]
                 [--server nostdb://HOST:PORT] [--credential-file PATH]
                 [--format table|json|jsonl|csv] [--read-only] [--interactive]
    nostdb server ping --server nostdb://HOST:PORT [--credential-file PATH]
    nostdb database create NAME|list|inspect NAME|rename NAME NEW_NAME
                    |drop NAME --confirm NAME|snapshot NAME --output PATH
                    |restore NAME --file PATH|export-logical NAME --output PATH
                    |import-logical NAME --file PATH
                    --server nostdb://HOST:PORT [--credential-file PATH]
                    [--format table|json|jsonl|csv]
    nostdb sync --project PATH [--database PATH] [--format table|json|jsonl|csv]
    nostdb format --file PATH [--project PATH | --source-version VERSION] [--check]
    nostdb check|inspect|stats|schema|unresolved (--database PATH | --project PATH)
                 [--format table|json|jsonl|csv]
    nostdb imports|warnings --project PATH [--format table|json|jsonl|csv]
    nostdb doctor --project PATH [--database PATH] [--format table|json|jsonl|csv]
    nostdb --help
    nostdb --version

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
    database_explicit: bool,
    project: Option<PathBuf>,
    format: OutputFormat,
}

#[derive(Debug)]
struct QueryOptions {
    common: CommonOptions,
    query: Option<String>,
    file: Option<PathBuf>,
    read_only: bool,
    interactive: bool,
    remote: Option<RemoteOptions>,
}

#[derive(Debug)]
struct RemoteOptions {
    server: String,
    credential_file: Option<PathBuf>,
}

#[derive(Debug)]
enum DatabaseCommand {
    Create(String),
    List,
    Inspect(String),
    Rename { name: String, new_name: String },
    Drop { name: String, confirm_name: String },
    Snapshot { name: String, output: PathBuf },
    Restore { name: String, file: PathBuf },
    ExportLogical { name: String, output: PathBuf },
    ImportLogical { name: String, file: PathBuf },
}

#[derive(Debug)]
struct DatabaseOptions {
    command: DatabaseCommand,
    remote: RemoteOptions,
    format: OutputFormat,
}

#[derive(Debug)]
struct FormatOptions {
    file: PathBuf,
    project: Option<PathBuf>,
    source_version: Option<u32>,
    check: bool,
}

#[derive(Debug)]
struct ProjectOptions {
    project: PathBuf,
    format: OutputFormat,
}

#[derive(Debug)]
struct InitOptions {
    project: PathBuf,
    database: PathBuf,
    allow_nonempty: bool,
    format: OutputFormat,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::from(EXIT_SUCCESS),
        Err(error) => {
            eprintln!("nostdb: {}", error.message);
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
        println!("nostdb {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let command = arguments.remove(0);
    if matches!(arguments.as_slice(), [argument] if matches!(argument.as_str(), "-h" | "--help")) {
        println!("{}", command_help(&command)?);
        return Ok(());
    }
    match command.as_str() {
        "init" => run_init(parse_init(arguments)?),
        "update" => run_update(parse_project(arguments)?),
        "query" => run_query(parse_query(arguments)?),
        "server" => run_server(parse_server(arguments)?),
        "database" => run_database(parse_database(arguments)?),
        "sync" => run_sync(parse_common(arguments, true)?),
        "format" => run_format(parse_format(arguments)?),
        "check" => run_check(parse_common(arguments, false)?),
        "doctor" => run_doctor(parse_common(arguments, true)?),
        "inspect" => run_inspect(parse_common(arguments, false)?),
        "stats" => run_stats(parse_common(arguments, false)?),
        "schema" => run_schema(parse_common(arguments, false)?),
        "unresolved" => run_unresolved(parse_common(arguments, false)?),
        "imports" => run_imports(parse_project(arguments)?),
        "warnings" => run_warnings(parse_project(arguments)?),
        _ => Err(CliError::usage(format!(
            "unknown command `{command}`\n\n{HELP}"
        ))),
    }
}

fn command_help(command: &str) -> Result<String, CliError> {
    let usage = match command {
        "init" => format!(
            "Usage: nostdb init --project PATH [--database FILE] [--allow-nonempty] [--format {MACHINE_FORMATS}]\n\nCreates a guarded NDB-only project with .nostdb/settings.json and .nostdb/root.nostdb by default. FILE must be a filename ending in .nostdb. It refuses an existing .nostdb directory and nonempty destinations unless --allow-nonempty is explicit."
        ),
        "update" => format!(
            "Usage: nostdb update --project PATH [--format {MACHINE_FORMATS}]\n\nSynchronizes linked child projects before the selected project so every configured root database is current."
        ),
        "query" => format!(
            "Usage: nostdb query [QUERY] [--file PATH] [--database PATH|NAME] [--project PATH]\n       [--server nostdb://HOST:PORT] [--credential-file PATH]\n       [--format {MACHINE_FORMATS}] [--read-only] [--interactive]\n\nProject queries use the database declared by .nostdb/settings.json. --read-only rejects every mutating statement before execution. Use jsonl for streaming output; multi-statement json is one array and multi-statement csv is rejected."
        ),
        "server" => {
            "Usage: nostdb server ping --server nostdb://HOST:PORT [--credential-file PATH]"
                .to_owned()
        }
        "database" => format!(
            "Usage: nostdb database create NAME|list|inspect NAME|rename NAME NEW_NAME\n       |drop NAME --confirm NAME|snapshot NAME --output PATH\n       |restore NAME --file PATH|export-logical NAME --output PATH\n       |import-logical NAME --file PATH\n       --server nostdb://HOST:PORT [--credential-file PATH] [--format {MACHINE_FORMATS}]"
        ),
        "sync" => format!(
            "Usage: nostdb sync --project PATH [--database PATH] [--format {MACHINE_FORMATS}]"
        ),
        "format" => {
            "Usage: nostdb format --file PATH [--project PATH | --source-version VERSION] [--check]"
                .to_owned()
        }
        "check" | "inspect" | "stats" | "schema" | "unresolved" => {
            format!(
                "Usage: nostdb {command} (--database PATH | --project PATH) [--format {MACHINE_FORMATS}]"
            )
        }
        "imports" | "warnings" => {
            format!("Usage: nostdb {command} --project PATH [--format {MACHINE_FORMATS}]")
        }
        "doctor" => format!(
            "Usage: nostdb doctor --project PATH [--database PATH] [--format {MACHINE_FORMATS}]"
        ),
        _ => {
            return Err(CliError::usage(format!(
                "unknown command `{command}`\n\n{HELP}"
            )));
        }
    };
    Ok(usage)
}

fn parse_init(arguments: Vec<String>) -> Result<InitOptions, CliError> {
    let mut project = None;
    let mut database = None;
    let mut allow_nonempty = false;
    let mut format = OutputFormat::Table;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "-p" | "--project" => project = Some(value(&arguments, &mut index)?.into()),
            "-d" | "--database" => {
                database = Some(database_filename(value(&arguments, &mut index)?)?)
            }
            "--allow-nonempty" => allow_nonempty = true,
            "--format" => format = OutputFormat::parse(value(&arguments, &mut index)?)?,
            option => return Err(CliError::usage(format!("unknown option `{option}`"))),
        }
        index += 1;
    }
    Ok(InitOptions {
        project: project.ok_or_else(|| CliError::usage("--project is required"))?,
        database: database.unwrap_or_else(|| PathBuf::from(DEFAULT_DATABASE_FILE)),
        allow_nonempty,
        format,
    })
}

fn parse_format(arguments: Vec<String>) -> Result<FormatOptions, CliError> {
    let mut file = None;
    let mut project: Option<PathBuf> = None;
    let mut source_version = None;
    let mut check = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "-f" | "--file" => file = Some(value(&arguments, &mut index)?.into()),
            "-p" | "--project" => project = Some(value(&arguments, &mut index)?.into()),
            "--source-version" => {
                source_version = Some(value(&arguments, &mut index)?.parse().map_err(|_| {
                    CliError::usage("--source-version must be an unsigned integer")
                })?);
            }
            "--check" => check = true,
            option => return Err(CliError::usage(format!("unknown option `{option}`"))),
        }
        index += 1;
    }
    if project.is_some() && source_version.is_some() {
        return Err(CliError::usage(
            "--project and --source-version are mutually exclusive",
        ));
    }
    Ok(FormatOptions {
        file: file.ok_or_else(|| CliError::usage("--file is required"))?,
        project,
        source_version,
        check,
    })
}

fn parse_project(arguments: Vec<String>) -> Result<ProjectOptions, CliError> {
    let mut project = None;
    let mut format = OutputFormat::Table;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "-p" | "--project" => project = Some(value(&arguments, &mut index)?.into()),
            "--format" => format = OutputFormat::parse(value(&arguments, &mut index)?)?,
            option => return Err(CliError::usage(format!("unknown option `{option}`"))),
        }
        index += 1;
    }
    Ok(ProjectOptions {
        project: project.ok_or_else(|| CliError::usage("--project is required"))?,
        format,
    })
}

fn parse_query(arguments: Vec<String>) -> Result<QueryOptions, CliError> {
    let mut database: Option<PathBuf> = None;
    let mut database_explicit = false;
    let mut project: Option<PathBuf> = None;
    let mut format = OutputFormat::Table;
    let mut query = None;
    let mut file = None;
    let mut read_only = false;
    let mut interactive = false;
    let mut server = None;
    let mut credential_file = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "-d" | "--database" => {
                database = Some(value(&arguments, &mut index)?.into());
                database_explicit = true;
            }
            "-p" | "--project" => project = Some(value(&arguments, &mut index)?.into()),
            "--server" => server = Some(value(&arguments, &mut index)?.to_owned()),
            "--credential-file" => credential_file = Some(value(&arguments, &mut index)?.into()),
            "-f" | "--file" => file = Some(value(&arguments, &mut index)?.into()),
            "--format" => format = OutputFormat::parse(value(&arguments, &mut index)?)?,
            "--read-only" => read_only = true,
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
    if server.is_some() && !database_explicit {
        return Err(CliError::usage("remote query requires --database NAME"));
    }
    if server.is_some() && project.is_some() {
        return Err(CliError::usage(
            "remote query cannot use --project; import through the server lifecycle",
        ));
    }
    if server.is_none() && credential_file.is_some() {
        return Err(CliError::usage("--credential-file requires --server"));
    }
    let database = database.unwrap_or_else(|| {
        project.as_ref().map_or_else(
            || PathBuf::from(PROJECT_DIRECTORY).join(DEFAULT_DATABASE_FILE),
            |root| root.join(PROJECT_DIRECTORY).join(DEFAULT_DATABASE_FILE),
        )
    });
    Ok(QueryOptions {
        common: CommonOptions {
            database,
            database_explicit,
            project,
            format,
        },
        query,
        file,
        read_only,
        interactive,
        remote: server.map(|server| RemoteOptions {
            server,
            credential_file,
        }),
    })
}

fn parse_server(mut arguments: Vec<String>) -> Result<RemoteOptions, CliError> {
    if arguments.first().map(String::as_str) != Some("ping") {
        return Err(CliError::usage(
            "server command must be `nostdb server ping`",
        ));
    }
    arguments.remove(0);
    parse_remote_options(arguments)
}

fn parse_remote_options(arguments: Vec<String>) -> Result<RemoteOptions, CliError> {
    let mut server = None;
    let mut credential_file = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--server" => server = Some(value(&arguments, &mut index)?.to_owned()),
            "--credential-file" => credential_file = Some(value(&arguments, &mut index)?.into()),
            option => return Err(CliError::usage(format!("unknown remote option `{option}`"))),
        }
        index += 1;
    }
    Ok(RemoteOptions {
        server: server.ok_or_else(|| CliError::usage("--server is required"))?,
        credential_file,
    })
}

fn parse_database(mut arguments: Vec<String>) -> Result<DatabaseOptions, CliError> {
    if arguments.is_empty() {
        return Err(CliError::usage("database operation is required"));
    }
    let operation = arguments.remove(0);
    let mut server = None;
    let mut credential_file = None;
    let mut confirm = None;
    let mut output = None;
    let mut file = None;
    let mut format = OutputFormat::Table;
    let mut operands = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--server" => server = Some(value(&arguments, &mut index)?.to_owned()),
            "--credential-file" => credential_file = Some(value(&arguments, &mut index)?.into()),
            "--confirm" => confirm = Some(value(&arguments, &mut index)?.to_owned()),
            "--output" => output = Some(value(&arguments, &mut index)?.into()),
            "--file" => file = Some(value(&arguments, &mut index)?.into()),
            "--format" => format = OutputFormat::parse(value(&arguments, &mut index)?)?,
            option if option.starts_with('-') => {
                return Err(CliError::usage(format!(
                    "unknown database option `{option}`"
                )));
            }
            operand => operands.push(operand.to_owned()),
        }
        index += 1;
    }
    let exact = |count: usize| {
        if operands.len() == count {
            Ok(())
        } else {
            Err(CliError::usage(format!(
                "database {operation} expects {count} name argument(s)"
            )))
        }
    };
    let command = match operation.as_str() {
        "create" => {
            exact(1)?;
            DatabaseCommand::Create(operands.remove(0))
        }
        "list" => {
            exact(0)?;
            DatabaseCommand::List
        }
        "inspect" => {
            exact(1)?;
            DatabaseCommand::Inspect(operands.remove(0))
        }
        "rename" => {
            exact(2)?;
            DatabaseCommand::Rename {
                name: operands.remove(0),
                new_name: operands.remove(0),
            }
        }
        "drop" => {
            exact(1)?;
            DatabaseCommand::Drop {
                name: operands.remove(0),
                confirm_name: confirm
                    .clone()
                    .ok_or_else(|| CliError::usage("database drop requires --confirm NAME"))?,
            }
        }
        "snapshot" => {
            exact(1)?;
            DatabaseCommand::Snapshot {
                name: operands.remove(0),
                output: output
                    .clone()
                    .ok_or_else(|| CliError::usage("database snapshot requires --output PATH"))?,
            }
        }
        "restore" => {
            exact(1)?;
            DatabaseCommand::Restore {
                name: operands.remove(0),
                file: file
                    .clone()
                    .ok_or_else(|| CliError::usage("database restore requires --file PATH"))?,
            }
        }
        "export-logical" => {
            exact(1)?;
            DatabaseCommand::ExportLogical {
                name: operands.remove(0),
                output: output.clone().ok_or_else(|| {
                    CliError::usage("database export-logical requires --output PATH")
                })?,
            }
        }
        "import-logical" => {
            exact(1)?;
            DatabaseCommand::ImportLogical {
                name: operands.remove(0),
                file: file.clone().ok_or_else(|| {
                    CliError::usage("database import-logical requires --file PATH")
                })?,
            }
        }
        _ => {
            return Err(CliError::usage(format!(
                "unknown database operation `{operation}`"
            )));
        }
    };
    if !matches!(command, DatabaseCommand::Drop { .. }) && confirm.is_some() {
        return Err(CliError::usage("--confirm is valid only for database drop"));
    }
    if !matches!(
        command,
        DatabaseCommand::Snapshot { .. } | DatabaseCommand::ExportLogical { .. }
    ) && output.is_some()
    {
        return Err(CliError::usage(
            "--output is valid only for snapshot or export-logical",
        ));
    }
    if !matches!(
        command,
        DatabaseCommand::Restore { .. } | DatabaseCommand::ImportLogical { .. }
    ) && file.is_some()
    {
        return Err(CliError::usage(
            "--file is valid only for restore or import-logical",
        ));
    }
    Ok(DatabaseOptions {
        command,
        remote: RemoteOptions {
            server: server.ok_or_else(|| CliError::usage("--server is required"))?,
            credential_file,
        },
        format,
    })
}

fn parse_common(arguments: Vec<String>, project_required: bool) -> Result<CommonOptions, CliError> {
    let mut database = None;
    let mut database_explicit = false;
    let mut project: Option<PathBuf> = None;
    let mut format = OutputFormat::Table;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "-d" | "--database" => {
                database = Some(value(&arguments, &mut index)?.into());
                database_explicit = true;
            }
            "-p" | "--project" => project = Some(value(&arguments, &mut index)?.into()),
            "--format" => format = OutputFormat::parse(value(&arguments, &mut index)?)?,
            option => return Err(CliError::usage(format!("unknown option `{option}`"))),
        }
        index += 1;
    }
    if project_required && project.is_none() {
        return Err(CliError::usage("--project is required"));
    }
    let database = database
        .or_else(|| {
            project
                .as_ref()
                .map(|project| project.join(PROJECT_DIRECTORY).join(DEFAULT_DATABASE_FILE))
        })
        .ok_or_else(|| CliError::usage("--database is required"))?;
    Ok(CommonOptions {
        database,
        database_explicit,
        project,
        format,
    })
}

fn resolve_project_database(options: &mut CommonOptions) -> Result<(), CliError> {
    let Some(project) = options.project.as_deref() else {
        return Ok(());
    };
    let config =
        ProjectConfig::load(project).map_err(|error| CliError::project(error.to_string()))?;
    let configured = config.root_path();
    if options.database_explicit
        && lexical_absolute(&options.database)? != lexical_absolute(&configured)?
    {
        return Err(CliError::project(format!(
            "--database {} does not match .nostdb/settings.json root {}",
            options.database.display(),
            configured.display()
        )));
    }
    options.database = configured;
    Ok(())
}

fn lexical_absolute(path: &Path) -> Result<PathBuf, CliError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| {
                CliError::new(EXIT_IO, format!("cannot read current directory: {error}"))
            })?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    Ok(normalized)
}

fn value<'a>(arguments: &'a [String], index: &mut usize) -> Result<&'a str, CliError> {
    *index += 1;
    arguments
        .get(*index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage("option requires a value"))
}

fn database_filename(value: &str) -> Result<PathBuf, CliError> {
    let path = PathBuf::from(value);
    if path.components().count() != 1
        || path.extension().and_then(|extension| extension.to_str()) != Some("nostdb")
    {
        return Err(CliError::usage(
            "--database must be a filename ending in .nostdb",
        ));
    }
    Ok(path)
}

fn run_init(options: InitOptions) -> Result<(), CliError> {
    let project = lexical_absolute(&options.project)?;
    if project.parent().is_none() {
        return Err(CliError::project(format!(
            "refusing broad project root initialization: {}",
            project.display()
        )));
    }
    for variable in ["HOME", "USERPROFILE"] {
        if let Some(home) = env::var_os(variable) {
            let home = lexical_absolute(Path::new(&home))?;
            if project == home {
                return Err(CliError::project(format!(
                    "refusing broad project root initialization: {}",
                    project.display()
                )));
            }
        }
    }

    match fs::symlink_metadata(&project) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(CliError::project(format!(
                "project root must not be a symlink: {}",
                project.display()
            )));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(CliError::project(format!(
                "project root is not a directory: {}",
                project.display()
            )));
        }
        Ok(_) => {
            let nonempty = fs::read_dir(&project)
                .map_err(|error| {
                    CliError::new(
                        EXIT_IO,
                        format!("cannot inspect {}: {error}", project.display()),
                    )
                })?
                .next()
                .transpose()
                .map_err(|error| {
                    CliError::new(
                        EXIT_IO,
                        format!("cannot inspect {}: {error}", project.display()),
                    )
                })?
                .is_some();
            if nonempty && !options.allow_nonempty {
                return Err(CliError::project(format!(
                    "refusing to initialize a nonempty directory without --allow-nonempty: {}",
                    project.display()
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(&project).map_err(|error| {
                CliError::new(
                    EXIT_IO,
                    format!("cannot create {}: {error}", project.display()),
                )
            })?;
        }
        Err(error) => {
            return Err(CliError::new(
                EXIT_IO,
                format!("cannot inspect {}: {error}", project.display()),
            ));
        }
    }

    let storage = project.join(PROJECT_DIRECTORY);
    if storage.exists() {
        return Err(CliError::project(format!(
            "refusing to adopt existing {}",
            storage.display()
        )));
    }

    fs::create_dir(&storage).map_err(|error| {
        CliError::new(
            EXIT_IO,
            format!("cannot create {}: {error}", storage.display()),
        )
    })?;
    let settings = storage.join(SETTINGS_FILE);
    let database = storage.join(&options.database);
    let mut settings_bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "version": SETTINGS_VERSION,
        "database": {
            "root": options.database.to_string_lossy(),
            "links": [],
        },
        "source": {
            "version": NOST_LANGUAGE_VERSION,
            "enabled": false,
        },
    }))
    .map_err(|error| CliError::project(format!("cannot encode settings.json: {error}")))?;
    settings_bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&settings)
        .map_err(|error| {
            let _ = fs::remove_dir(&storage);
            let code = if error.kind() == io::ErrorKind::AlreadyExists {
                EXIT_PROJECT
            } else {
                EXIT_IO
            };
            CliError::new(
                code,
                format!("cannot create {}: {error}", settings.display()),
            )
        })?;
    if let Err(error) = file
        .write_all(&settings_bytes)
        .and_then(|()| file.sync_all())
    {
        drop(file);
        cleanup_init_files(&settings, &database, &storage);
        return Err(CliError::new(
            EXIT_IO,
            format!("cannot write {}: {error}", settings.display()),
        ));
    }
    drop(file);

    let created = EmbeddedDatabase::create(&database).map_err(|error| {
        cleanup_init_files(&settings, &database, &storage);
        CliError::database(error)
    })?;
    drop(created);
    let report = synchronize(&project).inspect_err(|_| {
        cleanup_init_files(&settings, &database, &storage);
    })?;
    emit_project_diagnostics(&report.diagnostics);

    render_table_data(
        &["settings", "database", "version", "source_enabled"],
        &[vec![
            QueryValue::String(settings.display().to_string()),
            QueryValue::String(database.display().to_string()),
            QueryValue::Integer(i64::from(SETTINGS_VERSION)),
            QueryValue::Boolean(false),
        ]],
        options.format,
        &mut io::stdout(),
    )
}

fn cleanup_init_files(settings: &Path, database: &Path, storage: &Path) {
    let _ = fs::remove_file(settings);
    for suffix in ["", ".lock", "-wal", "-shm", "-journal"] {
        let mut path = database.as_os_str().to_os_string();
        path.push(suffix);
        let _ = fs::remove_file(PathBuf::from(path));
    }
    let _ = fs::remove_dir(storage);
}

fn run_update(options: ProjectOptions) -> Result<(), CliError> {
    let project = lexical_absolute(&options.project)?;
    let mut visiting = BTreeSet::new();
    let mut completed = BTreeSet::new();
    let mut reports = Vec::new();
    update_project_recursive(&project, &mut visiting, &mut completed, &mut reports)?;
    let rows = reports
        .into_iter()
        .map(|(project, report)| {
            Ok(vec![
                QueryValue::String(project.display().to_string()),
                QueryValue::String(report.database_path.display().to_string()),
                QueryValue::String(sync_direction(report.direction).to_owned()),
                integer(report.database_generation)?,
                QueryValue::Integer(report.diagnostics.len() as i64),
            ])
        })
        .collect::<Result<Vec<_>, CliError>>()?;
    render_table_data(
        &["project", "database", "direction", "generation", "warnings"],
        &rows,
        options.format,
        &mut io::stdout(),
    )
}

fn update_project_recursive(
    project: &Path,
    visiting: &mut BTreeSet<PathBuf>,
    completed: &mut BTreeSet<PathBuf>,
    reports: &mut Vec<(PathBuf, ProjectSyncReport)>,
) -> Result<(), CliError> {
    let project = lexical_absolute(project)?;
    if completed.contains(&project) {
        return Ok(());
    }
    if !visiting.insert(project.clone()) {
        return Err(CliError::project(format!(
            "linked project cycle includes {}",
            project.display()
        )));
    }
    let config =
        ProjectConfig::load(&project).map_err(|error| CliError::project(error.to_string()))?;
    for link in &config.links {
        update_project_recursive(&project.join(&link.project), visiting, completed, reports)?;
    }
    let report = synchronize(&project)?;
    emit_project_diagnostics(&report.diagnostics);
    visiting.remove(&project);
    completed.insert(project.clone());
    reports.push((project, report));
    Ok(())
}

fn run_query(mut options: QueryOptions) -> Result<(), CliError> {
    if options.remote.is_some() {
        return run_remote_query(options);
    }
    resolve_project_database(&mut options.common)?;
    let interactive = options.interactive
        || (options.query.is_none() && options.file.is_none() && io::stdin().is_terminal());
    if interactive {
        synchronize_for_query(&options)?;
        let mut database = Some(open_or_create(&options.common.database)?);
        repl(&options, &mut database)?;
        if !options.read_only {
            database
                .as_mut()
                .expect("interactive query keeps the database open")
                .checkpoint()
                .map_err(CliError::database)?;
            drop(database);
            synchronize_for_query(&options)?;
        }
        return Ok(());
    }

    // Input and query preparation intentionally precede synchronization and
    // database creation, so a typo or unreadable file cannot leave an artifact.
    let statements = read_and_validate_statements(&options)?;
    validate_batch_format(options.common.format, statements.len())?;
    let has_write = statements
        .iter()
        .any(|statement| prepare_write(statement).is_ok());
    synchronize_for_query(&options)?;
    let mut database = Some(open_or_create(&options.common.database)?);
    let mut results = Vec::with_capacity(statements.len());
    for statement in statements {
        let result = execute_one(&options, &mut database, &statement)?;
        results.push(result);
    }
    if !options.read_only {
        database
            .as_mut()
            .expect("query execution keeps the database open")
            .checkpoint()
            .map_err(CliError::database)?;
    }
    drop(database);
    if has_write {
        synchronize_for_query(&options)?;
    }
    render_statement_batch(&results, options.common.format, &mut io::stdout())
}

fn run_remote_query(options: QueryOptions) -> Result<(), CliError> {
    let interactive = options.interactive
        || (options.query.is_none() && options.file.is_none() && io::stdin().is_terminal());
    let statements = if interactive {
        None
    } else {
        let statements = read_and_validate_statements(&options)?;
        validate_batch_format(options.common.format, statements.len())?;
        Some(statements)
    };
    let remote = options
        .remote
        .as_ref()
        .expect("checked by remote query dispatch");
    let database_name = options
        .common
        .database
        .to_str()
        .ok_or_else(|| CliError::usage("remote Database name must be UTF-8"))?
        .to_owned();
    let mut client = connect_remote(remote)?;
    expect_selected(client.request(ClientRequest::SelectDatabase {
        database: database_name,
    }))?;
    if interactive {
        return remote_repl(&options, &mut client);
    }
    let statements = statements.expect("non-interactive input was prepared before connecting");
    let mut results = Vec::with_capacity(statements.len());
    for statement in statements {
        let response = remote_request(
            &mut client,
            ClientRequest::Query {
                query: statement,
                parameters: Default::default(),
                read_only: options.read_only,
                stream: false,
                limits: None,
            },
        )?;
        let ServerResponse::Result { statement } = response else {
            return Err(CliError::new(
                EXIT_DATABASE,
                "server returned an unexpected query response",
            ));
        };
        results.push(statement);
    }
    render_wire_statement_batch(&results, options.common.format, &mut io::stdout())
}

fn read_and_validate_statements(options: &QueryOptions) -> Result<Vec<String>, CliError> {
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
    for (index, statement) in statements.iter().enumerate() {
        validate_statement(statement, options.read_only, index, statements.len())?;
    }
    Ok(statements)
}

fn validate_statement(
    statement: &str,
    read_only: bool,
    index: usize,
    statement_count: usize,
) -> Result<(), CliError> {
    let read_error = match prepare(statement) {
        Ok(_) => return Ok(()),
        Err(error) => error,
    };
    if prepare_write(statement).is_ok() {
        if read_only {
            let prefix = (statement_count > 1).then(|| format!("statement {}: ", index + 1));
            return Err(CliError::new(
                EXIT_QUERY,
                format!(
                    "{}read-only mode rejects mutating statements",
                    prefix.unwrap_or_default()
                ),
            ));
        }
        return Ok(());
    }
    let message = if statement_count == 1 {
        read_error.to_string()
    } else {
        format!("statement {}: {read_error}", index + 1)
    };
    Err(CliError::new(EXIT_QUERY, message))
}

fn validate_batch_format(format: OutputFormat, statement_count: usize) -> Result<(), CliError> {
    if format == OutputFormat::Csv && statement_count > 1 {
        return Err(CliError::usage(
            "--format csv supports one statement per invocation because result schemas may differ; use --format jsonl for multi-statement input",
        ));
    }
    Ok(())
}

fn synchronize_for_query(options: &QueryOptions) -> Result<(), CliError> {
    if let Some(project) = &options.common.project {
        let report = synchronize(project)?;
        emit_project_diagnostics(&report.diagnostics);
    }
    Ok(())
}

fn remote_repl(options: &QueryOptions, client: &mut Client) -> Result<(), CliError> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let mut line = String::new();
    let mut buffer = String::new();
    loop {
        write!(
            stderr,
            "{}",
            if buffer.is_empty() {
                "nostdb> "
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
            let request = match trimmed {
                ":quit" | ":q" => break,
                ":help" => {
                    writeln!(stdout, ":help :ping :begin :commit :rollback :quit")
                        .map_err(output_error)?;
                    continue;
                }
                ":ping" => ClientRequest::Ping,
                ":begin" => ClientRequest::Begin,
                ":commit" => ClientRequest::Commit,
                ":rollback" => ClientRequest::Rollback,
                _ => {
                    writeln!(stderr, "error: unknown remote REPL command `{trimmed}`")
                        .map_err(output_error)?;
                    continue;
                }
            };
            match remote_request(client, request) {
                Ok(ServerResponse::Pong) => {
                    writeln!(stderr, "server is ready").map_err(output_error)?
                }
                Ok(ServerResponse::Transaction { state, results }) => {
                    for statement in results {
                        render_wire_statement(&statement, options.common.format, &mut stdout)?;
                    }
                    writeln!(stderr, "{state}").map_err(output_error)?;
                }
                Ok(_) => writeln!(stderr, "server acknowledged request").map_err(output_error)?,
                Err(error) => writeln!(stderr, "error: {}", error.message).map_err(output_error)?,
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
            if let Err(error) = validate_statement(&statement, options.read_only, 0, 1) {
                writeln!(stderr, "error: {}", error.message).map_err(output_error)?;
                continue;
            }
            match remote_request(
                client,
                ClientRequest::Query {
                    query: statement,
                    parameters: Default::default(),
                    read_only: options.read_only,
                    stream: false,
                    limits: None,
                },
            ) {
                Ok(ServerResponse::Result { statement }) => {
                    render_wire_statement(&statement, options.common.format, &mut stdout)?
                }
                Ok(ServerResponse::Queued { .. }) => {
                    writeln!(stderr, "queued").map_err(output_error)?
                }
                Ok(_) => writeln!(stderr, "unexpected server response").map_err(output_error)?,
                Err(error) => writeln!(stderr, "error: {}", error.message).map_err(output_error)?,
            }
        }
    }
    Ok(())
}

fn run_server(options: RemoteOptions) -> Result<(), CliError> {
    let mut client = connect_remote(&options)?;
    let response = remote_request(&mut client, ClientRequest::Ping)?;
    if !matches!(response, ServerResponse::Pong) {
        return Err(CliError::new(
            EXIT_DATABASE,
            "server returned an unexpected ping response",
        ));
    }
    render_table_data(
        &["server", "status"],
        &[vec![
            QueryValue::String(options.server),
            QueryValue::String("ready".to_owned()),
        ]],
        OutputFormat::Table,
        &mut io::stdout(),
    )
}

fn run_database(options: DatabaseOptions) -> Result<(), CliError> {
    let mut client = connect_remote(&options.remote)?;
    match options.command {
        DatabaseCommand::Create(name) => {
            let response = remote_request(&mut client, ClientRequest::DatabaseCreate { name })?;
            let ServerResponse::DatabaseCreated { database } = response else {
                return unexpected_database_response();
            };
            render_database_summaries(&[database], options.format)
        }
        DatabaseCommand::List => {
            let response = remote_request(&mut client, ClientRequest::DatabaseList)?;
            let ServerResponse::DatabaseList { databases } = response else {
                return unexpected_database_response();
            };
            render_database_summaries(&databases, options.format)
        }
        DatabaseCommand::Inspect(name) => {
            let response = remote_request(
                &mut client,
                ClientRequest::DatabaseInspect { database: name },
            )?;
            let ServerResponse::DatabaseInfo { database } = response else {
                return unexpected_database_response();
            };
            render_table_data(
                &[
                    "id",
                    "name",
                    "state",
                    "ndb_format",
                    "generation",
                    "checksum",
                    "healthy",
                    "schemas",
                    "nodes",
                    "edges",
                ],
                &[vec![
                    QueryValue::String(database.summary.id),
                    QueryValue::String(database.summary.name),
                    QueryValue::String(database.summary.state),
                    QueryValue::Integer(i64::from(database.ndb_format_version)),
                    integer(database.generation)?,
                    QueryValue::String(database.logical_checksum),
                    QueryValue::Boolean(database.healthy),
                    integer(database.schemas)?,
                    integer(database.nodes)?,
                    integer(database.edges)?,
                ]],
                options.format,
                &mut io::stdout(),
            )
        }
        DatabaseCommand::Rename { name, new_name } => {
            let response = remote_request(
                &mut client,
                ClientRequest::DatabaseRename {
                    database: name,
                    new_name,
                },
            )?;
            let ServerResponse::DatabaseRenamed { database } = response else {
                return unexpected_database_response();
            };
            render_database_summaries(&[database], options.format)
        }
        DatabaseCommand::Drop { name, confirm_name } => {
            let response = remote_request(
                &mut client,
                ClientRequest::DatabaseDrop {
                    database: name,
                    confirm_name,
                },
            )?;
            let ServerResponse::DatabaseDropped { database_id, name } = response else {
                return unexpected_database_response();
            };
            render_table_data(
                &["id", "name", "state"],
                &[vec![
                    QueryValue::String(database_id),
                    QueryValue::String(name),
                    QueryValue::String("dropped".to_owned()),
                ]],
                options.format,
                &mut io::stdout(),
            )
        }
        DatabaseCommand::Snapshot { name, output } => {
            export_snapshot(&mut client, &name, &output)?;
            render_table_data(
                &["database", "output", "state"],
                &[vec![
                    QueryValue::String(name),
                    QueryValue::String(output.display().to_string()),
                    QueryValue::String("snapshot_written".to_owned()),
                ]],
                options.format,
                &mut io::stdout(),
            )
        }
        DatabaseCommand::Restore { name, file } => {
            import_snapshot(&mut client, &name, &file)?;
            render_table_data(
                &["database", "input", "state"],
                &[vec![
                    QueryValue::String(name),
                    QueryValue::String(file.display().to_string()),
                    QueryValue::String("restored".to_owned()),
                ]],
                options.format,
                &mut io::stdout(),
            )
        }
        DatabaseCommand::ExportLogical { name, output } => {
            let response = remote_request(
                &mut client,
                ClientRequest::LogicalExport {
                    database: name.clone(),
                },
            )?;
            let ServerResponse::LogicalPackage { package } = response else {
                return unexpected_database_response();
            };
            let mut bytes = serde_json::to_vec_pretty(&package)
                .map_err(|error| CliError::new(EXIT_IO, error.to_string()))?;
            bytes.push(b'\n');
            write_new_output(&output, &bytes)?;
            render_table_data(
                &["database", "output", "state"],
                &[vec![
                    QueryValue::String(name),
                    QueryValue::String(output.display().to_string()),
                    QueryValue::String("logical_package_written".to_owned()),
                ]],
                options.format,
                &mut io::stdout(),
            )
        }
        DatabaseCommand::ImportLogical { name, file } => {
            let bytes = fs::read(&file).map_err(|error| {
                CliError::new(EXIT_IO, format!("cannot read {}: {error}", file.display()))
            })?;
            let package = serde_json::from_slice(&bytes)
                .map_err(|error| CliError::usage(format!("invalid logical package: {error}")))?;
            let response = remote_request(
                &mut client,
                ClientRequest::LogicalImport {
                    database: name.clone(),
                    package,
                },
            )?;
            let ServerResponse::LogicalImported { modules } = response else {
                return unexpected_database_response();
            };
            render_table_data(
                &["database", "modules", "state"],
                &[vec![
                    QueryValue::String(name),
                    integer(modules)?,
                    QueryValue::String("imported".to_owned()),
                ]],
                options.format,
                &mut io::stdout(),
            )
        }
    }
}

fn execute_one(
    options: &QueryOptions,
    database: &mut Option<EmbeddedDatabase>,
    statement: &str,
) -> Result<StatementResult, CliError> {
    validate_statement(statement, options.read_only, 0, 1)?;
    let parameters = Parameters::new();
    database
        .as_mut()
        .expect("query execution keeps the database open")
        .execute(statement, &parameters)
        .map_err(CliError::database)
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

fn synchronize(project: &Path) -> Result<ProjectSyncReport, CliError> {
    ProjectSynchronizer::default()
        .sync(project)
        .map_err(map_project_sync)
}

fn map_project_sync(error: ProjectSyncError) -> CliError {
    match &error {
        ProjectSyncError::Source(nostdb_engine::SyncError::Compile(error)) => {
            CliError::project(compile_error_message(error))
        }
        ProjectSyncError::Config(_) | ProjectSyncError::Source(_) => {
            CliError::project(error.to_string())
        }
        ProjectSyncError::Database(_) | ProjectSyncError::Export(_) => {
            CliError::new(EXIT_DATABASE, error.to_string())
        }
        ProjectSyncError::Conflict { .. } => CliError::new(EXIT_CONFLICT, error.to_string()),
        ProjectSyncError::Io(_) => CliError::new(EXIT_IO, error.to_string()),
    }
}

fn compile_error_message(error: &CompileError) -> String {
    match error {
        CompileError::Diagnostics(diagnostics) => format!(
            "project compilation failed with {} error diagnostic(s):\n{}",
            diagnostics.len(),
            format_project_diagnostics(diagnostics, None)
        ),
        _ => error.to_string(),
    }
}

fn format_project_diagnostics(
    diagnostics: &[ProjectDiagnostic],
    module_override: Option<&Path>,
) -> String {
    diagnostics
        .iter()
        .map(|value| {
            let module = module_override.map_or_else(
                || {
                    value
                        .module
                        .as_deref()
                        .map_or_else(|| "<project>".to_owned(), |path| path.display().to_string())
                },
                |path| path.display().to_string(),
            );
            let range = value.diagnostic.primary().map_or_else(
                || "bytes -".to_owned(),
                |range| format!("bytes {}..{}", range.start(), range.end()),
            );
            format!(
                "{module}:{range}: {} {}: {}",
                value.diagnostic.code(),
                value.diagnostic.severity(),
                value.diagnostic.message()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn emit_project_diagnostics(diagnostics: &[ProjectDiagnostic]) {
    if !diagnostics.is_empty() {
        eprintln!("{}", format_project_diagnostics(diagnostics, None));
    }
}

fn diagnose_source_for_format(
    source: &[u8],
    source_version: u32,
) -> Option<Vec<ProjectDiagnostic>> {
    let sequence = FORMAT_DIAGNOSTIC_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = env::temp_dir().join(format!(
        "nostdb-format-diagnostics-{}-{sequence}",
        std::process::id()
    ));
    if fs::create_dir(&directory).is_err() {
        return None;
    }
    let storage = directory.join(PROJECT_DIRECTORY);
    let settings_path = storage.join(SETTINGS_FILE);
    let source_path = storage.join("main.nost");
    let settings = format!(
        "{{\"version\":1,\"database\":{{\"root\":\"root.nostdb\",\"links\":[]}},\"source\":{{\"version\":{source_version},\"enabled\":true,\"modules\":{{\"main.nost\":\"00000000-0000-0000-0000-000000000001\"}}}}}}\n"
    );
    let setup = fs::create_dir(&storage)
        .and_then(|()| fs::write(&settings_path, settings))
        .and_then(|()| fs::write(&source_path, source));
    let result = if setup.is_ok() {
        ProjectCompiler::new().compile(&directory)
    } else {
        let _ = fs::remove_dir_all(&directory);
        return None;
    };
    let _ = fs::remove_dir_all(&directory);
    match result {
        Err(CompileError::Diagnostics(diagnostics)) => Some(diagnostics),
        Ok(compiled) if !compiled.diagnostics.is_empty() => Some(compiled.diagnostics),
        Ok(_) | Err(_) => None,
    }
}

fn run_sync(mut options: CommonOptions) -> Result<(), CliError> {
    resolve_project_database(&mut options)?;
    let report = synchronize(options.project.as_deref().expect("required by parser"))?;
    emit_project_diagnostics(&report.diagnostics);
    let source_updated = match report.source_updated_at_unix_ms {
        Some(value) => integer(value)?,
        None => QueryValue::Null,
    };
    let rows = vec![vec![
        QueryValue::String(sync_direction(report.direction).to_owned()),
        QueryValue::String(report.database_path.display().to_string()),
        integer(report.database_generation)?,
        integer(report.database_updated_at_unix_ms)?,
        source_updated,
        QueryValue::Integer(i64::from(report.attempts)),
        QueryValue::Integer(report.diagnostics.len() as i64),
    ]];
    render_table_data(
        &[
            "direction",
            "database",
            "database_generation",
            "database_updated_at_unix_ms",
            "source_updated_at_unix_ms",
            "attempts",
            "warnings",
        ],
        &rows,
        options.format,
        &mut io::stdout(),
    )
}

fn sync_direction(direction: ProjectSyncDirection) -> &'static str {
    match direction {
        ProjectSyncDirection::None => "none",
        ProjectSyncDirection::DatabaseToSource => "database_to_source",
        ProjectSyncDirection::SourceToDatabase => "source_to_database",
        ProjectSyncDirection::SourcesRemoved => "sources_removed",
    }
}

fn run_format(options: FormatOptions) -> Result<(), CliError> {
    let source = fs::read(&options.file).map_err(|error| {
        CliError::new(
            EXIT_IO,
            format!("cannot read {}: {error}", options.file.display()),
        )
    })?;
    let source_version = if let Some(project) = options.project {
        ProjectConfig::load(project)
            .map_err(|error| CliError::project(error.to_string()))?
            .source_version
    } else {
        options.source_version.unwrap_or(NOST_LANGUAGE_VERSION)
    };
    let formatted = format_source(&source, source_version).map_err(|error| {
        let diagnostics = diagnose_source_for_format(&source, source_version);
        let detail = diagnostics
            .as_deref()
            .filter(|values| !values.is_empty())
            .map(|values| format_project_diagnostics(values, Some(options.file.as_path())));
        CliError::project(match detail {
            Some(detail) => format!("{error}\n{detail}"),
            None => error.to_string(),
        })
    })?;
    if options.check {
        if source != formatted.as_bytes() {
            return Err(CliError::project(format!(
                "{} is not canonically formatted",
                options.file.display()
            )));
        }
        return Ok(());
    }
    io::stdout()
        .write_all(formatted.as_bytes())
        .map_err(output_error)
}

fn run_check(mut options: CommonOptions) -> Result<(), CliError> {
    resolve_project_database(&mut options)?;
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

fn run_inspect(mut options: CommonOptions) -> Result<(), CliError> {
    resolve_project_database(&mut options)?;
    let database = EmbeddedDatabase::open(&options.database).map_err(CliError::database)?;
    let info = database.info().map_err(CliError::database)?;
    render_table_data(
        &[
            "ndb_format",
            "schema_revision",
            "generation",
            "updated_at_unix_ms",
            "logical_checksum",
            "source_enabled",
        ],
        &[vec![
            QueryValue::Integer(i64::from(info.ndb_format_version)),
            QueryValue::Integer(i64::from(info.schema_revision)),
            integer(info.generation)?,
            integer(info.updated_at_unix_ms)?,
            QueryValue::String(format!("{:016x}", info.logical_checksum)),
            QueryValue::Boolean(info.source_enabled),
        ]],
        options.format,
        &mut io::stdout(),
    )
}

fn run_stats(mut options: CommonOptions) -> Result<(), CliError> {
    resolve_project_database(&mut options)?;
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

fn run_schema(mut options: CommonOptions) -> Result<(), CliError> {
    resolve_project_database(&mut options)?;
    let database = EmbeddedDatabase::open(&options.database).map_err(CliError::database)?;
    let rows = schema_rows(database.schemas().map_err(CliError::database)?);
    render_table_data(
        &[
            "identity",
            "state",
            "property",
            "property_type",
            "constraints",
        ],
        &rows,
        options.format,
        &mut io::stdout(),
    )
}

fn schema_rows(values: Vec<SchemaInfo>) -> Vec<Vec<QueryValue>> {
    let mut rows = Vec::new();
    for schema in values {
        if schema.properties.is_empty() {
            rows.push(vec![
                QueryValue::String(schema.identity),
                QueryValue::String(schema.state),
                QueryValue::Null,
                QueryValue::Null,
                QueryValue::Integer(schema.constraints as i64),
            ]);
            continue;
        }
        rows.extend(schema.properties.into_iter().map(|property| {
            vec![
                QueryValue::String(schema.identity.clone()),
                QueryValue::String(schema.state.clone()),
                QueryValue::String(property.name),
                QueryValue::String(property.property_type),
                QueryValue::Integer(schema.constraints as i64),
            ]
        }));
    }
    rows
}

fn run_unresolved(mut options: CommonOptions) -> Result<(), CliError> {
    resolve_project_database(&mut options)?;
    let database = EmbeddedDatabase::open(&options.database).map_err(CliError::database)?;
    let rows = unresolved_rows(database.unresolved().map_err(CliError::database)?)?;
    render_table_data(
        &["kind", "internal_id", "identity", "state"],
        &rows,
        options.format,
        &mut io::stdout(),
    )
}

fn unresolved_rows(values: Vec<UnresolvedInfo>) -> Result<Vec<Vec<QueryValue>>, CliError> {
    values
        .into_iter()
        .map(|value| {
            let internal_id = match value.internal_id {
                None => QueryValue::Null,
                Some(id) => QueryValue::Integer(i64::try_from(id).map_err(|_| {
                    CliError::new(
                        EXIT_DATABASE,
                        "internal Node ID exceeds the query Integer range",
                    )
                })?),
            };
            Ok(vec![
                QueryValue::String(value.kind),
                internal_id,
                QueryValue::String(value.identity),
                QueryValue::String(value.state),
            ])
        })
        .collect()
}

fn run_imports(options: ProjectOptions) -> Result<(), CliError> {
    let config = ProjectConfig::load(&options.project)
        .map_err(|error| CliError::project(error.to_string()))?;
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
    render_table_data(
        &["module", "stable_id"],
        &rows,
        options.format,
        &mut io::stdout(),
    )
}

fn run_warnings(options: ProjectOptions) -> Result<(), CliError> {
    let compiled = ProjectCompiler::new()
        .compile(&options.project)
        .map_err(|error| CliError::project(compile_error_message(&error)))?;
    let rows = diagnostic_rows(compiled.diagnostics);
    render_table_data(
        &["module", "range", "code", "severity", "message"],
        &rows,
        options.format,
        &mut io::stdout(),
    )
}

fn run_doctor(mut options: CommonOptions) -> Result<(), CliError> {
    resolve_project_database(&mut options)?;
    let project = options.project.as_deref().expect("required by parser");
    let config =
        ProjectConfig::load(project).map_err(|error| CliError::project(error.to_string()))?;
    let database = EmbeddedDatabase::open(&options.database).map_err(CliError::database)?;
    let status = database.check().map_err(CliError::database)?;
    if !status.is_valid() {
        return Err(CliError::new(
            EXIT_DATABASE,
            "database integrity check failed",
        ));
    }
    if !config.source_enabled() {
        let synchronized = !config.has_materialized_sources()
            && !database.info().map_err(CliError::database)?.source_enabled;
        let sync_status = if synchronized {
            "ndb_only"
        } else {
            "source_visibility_pending_sync"
        };
        render_table_data(
            &[
                "project",
                "database",
                "synchronized",
                "sync_status",
                "warnings",
            ],
            &[vec![
                QueryValue::String("ok".to_owned()),
                QueryValue::String("ok".to_owned()),
                QueryValue::Boolean(synchronized),
                QueryValue::String(sync_status.to_owned()),
                QueryValue::Integer(0),
            ]],
            options.format,
            &mut io::stdout(),
        )?;
        return if synchronized {
            Ok(())
        } else {
            Err(CliError::project(
                "source visibility does not match `source.enabled: false`; run `nostdb sync --project PATH`",
            ))
        };
    }

    let mut compiler = ProjectCompiler::new();
    let compiled = compiler
        .compile(project)
        .map_err(|error| CliError::project(compile_error_message(&error)))?;
    let info = database.info().map_err(CliError::database)?;
    let manifest = database.sync_manifest().map_err(CliError::database)?;
    let mut current_modules = compiled
        .source_hashes
        .iter()
        .map(|(path, hash)| {
            config
                .modules
                .get(path)
                .copied()
                .map(|module_id| (module_id, hash.as_bytes()))
                .ok_or_else(|| {
                    CliError::project(format!(
                        "compiled module {} has no Stable Module ID mapping",
                        path.display()
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    current_modules.sort_by_key(|(module_id, _)| *module_id);
    let (synchronized, sync_status) =
        manifest
            .as_ref()
            .map_or((false, "no_sync_baseline"), |manifest| {
                let stored_modules = manifest
                    .modules
                    .iter()
                    .map(|module| (module.module_id, module.content_hash))
                    .collect::<Vec<_>>();
                if manifest.semantic_hash == compiled.semantic_hash.as_bytes()
                    && manifest.database_generation == info.generation
                    && stored_modules == current_modules
                {
                    (true, "synchronized")
                } else {
                    (false, "source_drift")
                }
            });
    render_table_data(
        &[
            "project",
            "database",
            "synchronized",
            "sync_status",
            "warnings",
        ],
        &[vec![
            QueryValue::String("ok".to_owned()),
            QueryValue::String("ok".to_owned()),
            QueryValue::Boolean(synchronized),
            QueryValue::String(sync_status.to_owned()),
            QueryValue::Integer(compiled.diagnostics.len() as i64),
        ]],
        options.format,
        &mut io::stdout(),
    )?;
    match sync_status {
        "synchronized" => Ok(()),
        "no_sync_baseline" => Err(CliError::new(
            EXIT_DATABASE,
            "database has no `.nost` synchronization baseline for this project; run `nostdb sync --project PATH`",
        )),
        _ => Err(CliError::project(
            "source files or project identity differ from the database synchronization manifest; run `nostdb sync` before using this database",
        )),
    }
}

fn repl(options: &QueryOptions, database: &mut Option<EmbeddedDatabase>) -> Result<(), CliError> {
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
                "nostdb> "
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
            match handle_admin(
                trimmed,
                options,
                database,
                &mut transaction,
                &mut stdout,
                &mut stderr,
            ) {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) if error.code == EXIT_USAGE => {
                    writeln!(stderr, "error: {}", error.message).map_err(output_error)?;
                }
                Err(error) => return Err(error),
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
                match validate_statement(&statement, options.read_only, 0, 1) {
                    Ok(()) => {
                        pending.push((statement, Parameters::new()));
                        writeln!(stderr, "queued").map_err(output_error)?;
                    }
                    Err(error) => {
                        writeln!(stderr, "error: {}", error.message).map_err(output_error)?;
                    }
                }
            } else {
                match execute_one(options, database, &statement) {
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
    database: &mut Option<EmbeddedDatabase>,
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
            let info = database
                .as_ref()
                .expect("REPL keeps the database open")
                .info()
                .map_err(CliError::database)?;
            render_table_data(
                &["generation", "source_enabled"],
                &[vec![
                    QueryValue::Integer(info.generation as i64),
                    QueryValue::Boolean(info.source_enabled),
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
            let mut open = database.take().expect("REPL keeps the database open");
            open.checkpoint().map_err(CliError::database)?;
            drop(open);
            let synchronization = synchronize(project);
            *database = Some(
                EmbeddedDatabase::open(&options.common.database).map_err(CliError::database)?,
            );
            let report = synchronization?;
            if !report.diagnostics.is_empty() {
                writeln!(
                    stderr,
                    "{}",
                    format_project_diagnostics(&report.diagnostics, None)
                )
                .map_err(output_error)?;
            }
            writeln!(stderr, "synchronized").map_err(output_error)?;
        }
        ":schema" => {
            let rows = schema_rows(
                database
                    .as_ref()
                    .expect("REPL keeps the database open")
                    .schemas()
                    .map_err(CliError::database)?,
            );
            render_table_data(
                &[
                    "identity",
                    "state",
                    "property",
                    "property_type",
                    "constraints",
                ],
                &rows,
                options.common.format,
                stdout,
            )?;
        }
        ":unresolved" => {
            let values = database
                .as_ref()
                .expect("REPL keeps the database open")
                .unresolved()
                .map_err(CliError::database)?;
            let rows = unresolved_rows(values)?;
            render_table_data(
                &["kind", "internal_id", "identity", "state"],
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
            match database
                .as_mut()
                .expect("REPL keeps the database open")
                .execute_transaction(&statements)
            {
                Ok(results) => {
                    for result in results {
                        render_statement(&result, options.common.format, stdout)?;
                    }
                    database
                        .as_mut()
                        .expect("REPL keeps the database open")
                        .checkpoint()
                        .map_err(CliError::database)?;
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
        .map_err(|error| CliError::project(compile_error_message(&error)))?;
    let rows = diagnostic_rows(compiled.diagnostics);
    render_table_data(
        &["module", "range", "code", "severity", "message"],
        &rows,
        options.common.format,
        output,
    )
}

fn diagnostic_rows(diagnostics: Vec<ProjectDiagnostic>) -> Vec<Vec<QueryValue>> {
    diagnostics
        .into_iter()
        .map(|value| {
            let range = value.diagnostic.primary().map_or_else(
                || "-".to_owned(),
                |range| format!("{}..{}", range.start(), range.end()),
            );
            vec![
                QueryValue::String(
                    value
                        .module
                        .map_or_else(|| "<project>".to_owned(), |path| path.display().to_string()),
                ),
                QueryValue::String(range),
                QueryValue::String(value.diagnostic.code().to_string()),
                QueryValue::String(value.diagnostic.severity().to_string()),
                QueryValue::String(value.diagnostic.message().to_owned()),
            ]
        })
        .collect()
}

fn connect_remote(options: &RemoteOptions) -> Result<Client, CliError> {
    let credential = if let Some(path) = &options.credential_file {
        fs::read_to_string(path).map_err(|error| {
            CliError::new(
                EXIT_IO,
                format!("cannot read credential file {}: {error}", path.display()),
            )
        })?
    } else {
        env::var("NOSTDB_CREDENTIAL")
            .map_err(|_| CliError::usage("set NOSTDB_CREDENTIAL or pass --credential-file PATH"))?
    };
    let credential = credential.trim_end_matches(['\r', '\n']);
    if credential.len() < 32 || credential.chars().any(char::is_whitespace) {
        return Err(CliError::usage(
            "credential must be one non-whitespace token of at least 32 characters",
        ));
    }
    Client::connect(&options.server, credential, "nostdb-cli").map_err(map_remote_error)
}

fn remote_request(client: &mut Client, request: ClientRequest) -> Result<ServerResponse, CliError> {
    client.request(request).map_err(map_remote_error)
}

fn expect_selected(result: Result<ServerResponse, ClientError>) -> Result<(), CliError> {
    match result.map_err(map_remote_error)? {
        ServerResponse::DatabaseSelected { .. } => Ok(()),
        _ => Err(CliError::new(
            EXIT_DATABASE,
            "server returned an unexpected Database-selection response",
        )),
    }
}

fn map_remote_error(error: ClientError) -> CliError {
    let code = match &error {
        ClientError::Server {
            code:
                RemoteErrorCode::QueryError
                | RemoteErrorCode::ResourceLimit
                | RemoteErrorCode::Cancelled,
            ..
        } => EXIT_QUERY,
        ClientError::Protocol(_) => EXIT_USAGE,
        ClientError::Io(_) | ClientError::Server { .. } => EXIT_DATABASE,
    };
    CliError::new(code, error.to_string())
}

fn frame_response(
    frame: nostdb_client::ServerFrame,
    request_id: u64,
) -> Result<ServerResponse, CliError> {
    if frame.request_id != request_id {
        return Err(CliError::new(
            EXIT_DATABASE,
            format!(
                "database protocol returned response {} while waiting for {request_id}",
                frame.request_id
            ),
        ));
    }
    match frame.response {
        ServerResponse::Error {
            code,
            message,
            retryable,
        } => Err(map_remote_error(ClientError::Server {
            code,
            message,
            retryable,
        })),
        response => Ok(response),
    }
}

fn render_database_summaries(
    databases: &[nostdb_client::DatabaseSummary],
    format: OutputFormat,
) -> Result<(), CliError> {
    let rows = databases
        .iter()
        .map(|database| {
            vec![
                QueryValue::String(database.id.clone()),
                QueryValue::String(database.name.clone()),
                QueryValue::String(database.state.clone()),
            ]
        })
        .collect::<Vec<_>>();
    render_table_data(&["id", "name", "state"], &rows, format, &mut io::stdout())
}

fn unexpected_database_response<T>() -> Result<T, CliError> {
    Err(CliError::new(
        EXIT_DATABASE,
        "server returned an unexpected Database administration response",
    ))
}

fn export_snapshot(client: &mut Client, name: &str, output: &Path) -> Result<(), CliError> {
    if output.exists() {
        return Err(CliError::new(
            EXIT_IO,
            format!("refusing to replace existing output {}", output.display()),
        ));
    }
    let temporary = output.with_extension(format!("partial-{}", std::process::id()));
    let request_id = client
        .send(ClientRequest::SnapshotExport {
            database: name.to_owned(),
        })
        .map_err(map_remote_error)?;
    let result = (|| {
        let start = frame_response(client.read().map_err(map_remote_error)?, request_id)?;
        let ServerResponse::SnapshotStart { total_bytes } = start else {
            return Err(CliError::new(
                EXIT_DATABASE,
                "snapshot stream did not start correctly",
            ));
        };
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| {
                CliError::new(
                    EXIT_IO,
                    format!("cannot create {}: {error}", temporary.display()),
                )
            })?;
        let mut expected_sequence = 0_u64;
        let mut received = 0_u64;
        loop {
            match frame_response(client.read().map_err(map_remote_error)?, request_id)? {
                ServerResponse::SnapshotChunk { sequence, data } => {
                    if sequence != expected_sequence {
                        return Err(CliError::new(
                            EXIT_DATABASE,
                            "snapshot chunk sequence is not contiguous",
                        ));
                    }
                    let bytes = BASE64.decode(data).map_err(|error| {
                        CliError::new(EXIT_DATABASE, format!("invalid snapshot chunk: {error}"))
                    })?;
                    file.write_all(&bytes).map_err(|error| {
                        CliError::new(EXIT_IO, format!("cannot write snapshot: {error}"))
                    })?;
                    received = received.saturating_add(bytes.len() as u64);
                    expected_sequence += 1;
                }
                ServerResponse::SnapshotEnd { chunks } => {
                    if chunks != expected_sequence || received != total_bytes {
                        return Err(CliError::new(
                            EXIT_DATABASE,
                            "snapshot byte or chunk count does not match its declaration",
                        ));
                    }
                    file.sync_all().map_err(|error| {
                        CliError::new(EXIT_IO, format!("cannot persist snapshot: {error}"))
                    })?;
                    drop(file);
                    fs::rename(&temporary, output).map_err(|error| {
                        CliError::new(
                            EXIT_IO,
                            format!("cannot install {}: {error}", output.display()),
                        )
                    })?;
                    return Ok(());
                }
                _ => {
                    return Err(CliError::new(
                        EXIT_DATABASE,
                        "unexpected frame in snapshot stream",
                    ));
                }
            }
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn import_snapshot(client: &mut Client, name: &str, path: &Path) -> Result<(), CliError> {
    let mut file = fs::File::open(path).map_err(|error| {
        CliError::new(EXIT_IO, format!("cannot read {}: {error}", path.display()))
    })?;
    let total_bytes = file
        .metadata()
        .map_err(|error| CliError::new(EXIT_IO, error.to_string()))?
        .len();
    let response = remote_request(
        client,
        ClientRequest::SnapshotRestoreBegin {
            database: name.to_owned(),
            total_bytes,
        },
    )?;
    if !matches!(
        response,
        ServerResponse::SnapshotRestore { ref state, bytes: 0 } if state == "ready"
    ) {
        return unexpected_database_response();
    }
    let mut buffer = vec![0_u8; SNAPSHOT_CHUNK_BYTES];
    let mut sequence = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| CliError::new(EXIT_IO, format!("cannot read snapshot: {error}")))?;
        if read == 0 {
            break;
        }
        let response = remote_request(
            client,
            ClientRequest::SnapshotRestoreChunk {
                sequence,
                data: BASE64.encode(&buffer[..read]),
            },
        )?;
        if !matches!(
            response,
            ServerResponse::SnapshotRestore { ref state, .. } if state == "chunk_accepted"
        ) {
            return unexpected_database_response();
        }
        sequence += 1;
    }
    let response = remote_request(client, ClientRequest::SnapshotRestoreCommit)?;
    if matches!(
        response,
        ServerResponse::SnapshotRestore { ref state, bytes } if state == "restored" && bytes == total_bytes
    ) {
        Ok(())
    } else {
        unexpected_database_response()
    }
}

fn write_new_output(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            CliError::new(
                EXIT_IO,
                format!("cannot create output {}: {error}", path.display()),
            )
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            CliError::new(
                EXIT_IO,
                format!("cannot persist output {}: {error}", path.display()),
            )
        })
}

fn render_wire_statement(
    statement: &serde_json::Value,
    format: OutputFormat,
    output: &mut impl Write,
) -> Result<(), CliError> {
    match statement.get("kind").and_then(serde_json::Value::as_str) {
        Some("read") => render_wire_query(
            statement
                .get("result")
                .ok_or_else(|| CliError::new(EXIT_DATABASE, "read result is missing"))?,
            format,
            output,
        ),
        Some("write") => {
            let write = statement
                .get("write")
                .ok_or_else(|| CliError::new(EXIT_DATABASE, "write result is missing"))?;
            if let Some(result) = write.get("result").filter(|value| !value.is_null()) {
                render_wire_query(result, format, output)?;
                if format != OutputFormat::Table {
                    return Ok(());
                }
            }
            let summary = write
                .get("summary")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| CliError::new(EXIT_DATABASE, "write summary is missing"))?;
            let columns = [
                "nodes_created",
                "edges_created",
                "nodes_deleted",
                "edges_deleted",
                "properties_set",
                "properties_removed",
            ];
            let row = columns
                .iter()
                .map(|column| {
                    summary
                        .get(*column)
                        .cloned()
                        .map(json_query_value)
                        .ok_or_else(|| {
                            CliError::new(EXIT_DATABASE, "write summary field is missing")
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            render_table_data(&columns, &[row], format, output)
        }
        _ => Err(CliError::new(
            EXIT_DATABASE,
            "statement result has an unknown kind",
        )),
    }
}

fn render_wire_statement_batch(
    statements: &[serde_json::Value],
    format: OutputFormat,
    output: &mut impl Write,
) -> Result<(), CliError> {
    if format == OutputFormat::Json && statements.len() > 1 {
        write!(output, "[").map_err(output_error)?;
        for (index, statement) in statements.iter().enumerate() {
            if index > 0 {
                write!(output, ",").map_err(output_error)?;
            }
            let mut item = Vec::new();
            render_wire_statement(statement, OutputFormat::Json, &mut item)?;
            trim_line_ending(&mut item);
            output.write_all(&item).map_err(output_error)?;
        }
        return writeln!(output, "]").map_err(output_error);
    }
    for statement in statements {
        render_wire_statement(statement, format, output)?;
    }
    Ok(())
}

fn render_wire_query(
    result: &serde_json::Value,
    format: OutputFormat,
    output: &mut impl Write,
) -> Result<(), CliError> {
    let columns = result
        .get("columns")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::new(EXIT_DATABASE, "query columns are missing"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| CliError::new(EXIT_DATABASE, "query column is not text"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let rows = result
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::new(EXIT_DATABASE, "query rows are missing"))?
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or_else(|| CliError::new(EXIT_DATABASE, "query row is not an array"))
                .map(|values| values.iter().cloned().map(json_query_value).collect())
        })
        .collect::<Result<Vec<Vec<QueryValue>>, _>>()?;
    let columns = columns.iter().map(String::as_str).collect::<Vec<_>>();
    render_table_data(&columns, &rows, format, output)
}

fn json_query_value(value: serde_json::Value) -> QueryValue {
    match value {
        serde_json::Value::Null => QueryValue::Null,
        serde_json::Value::Bool(value) => QueryValue::Boolean(value),
        serde_json::Value::Number(value) => value
            .as_i64()
            .map(QueryValue::Integer)
            .or_else(|| value.as_f64().map(QueryValue::Float))
            .unwrap_or_else(|| QueryValue::String(value.to_string())),
        serde_json::Value::String(value) => QueryValue::String(value),
        serde_json::Value::Array(values) => {
            QueryValue::List(values.into_iter().map(json_query_value).collect())
        }
        serde_json::Value::Object(values) => QueryValue::Map(
            values
                .into_iter()
                .map(|(name, value)| (name, json_query_value(value)))
                .collect(),
        ),
    }
}

fn integer(value: u64) -> Result<QueryValue, CliError> {
    i64::try_from(value)
        .map(QueryValue::Integer)
        .map_err(|_| CliError::new(EXIT_DATABASE, "server integer exceeds CLI range"))
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

fn render_statement_batch(
    results: &[StatementResult],
    format: OutputFormat,
    output: &mut impl Write,
) -> Result<(), CliError> {
    if format == OutputFormat::Json && results.len() > 1 {
        write!(output, "[").map_err(output_error)?;
        for (index, result) in results.iter().enumerate() {
            if index > 0 {
                write!(output, ",").map_err(output_error)?;
            }
            let mut item = Vec::new();
            render_statement(result, OutputFormat::Json, &mut item)?;
            trim_line_ending(&mut item);
            output.write_all(&item).map_err(output_error)?;
        }
        return writeln!(output, "]").map_err(output_error);
    }
    for result in results {
        render_statement(result, format, output)?;
    }
    Ok(())
}

fn trim_line_ending(bytes: &mut Vec<u8>) {
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
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
        QueryValue::Bytes(value) => hex(value),
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
            let text = hex(value);
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
            write!(output, "{{\"id\":{},\"kind\":", edge.id.get()).map_err(output_error)?;
            write_json_string(
                output,
                match edge.kind {
                    EdgeKind::Directed => "directed",
                    EdgeKind::Undirected => "directionless",
                    EdgeKind::Bidirectional => "bidirectional",
                },
            )?;
            write!(
                output,
                ",\"source\":{},\"target\":{},\"type\":",
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

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
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

    #[test]
    fn remote_query_and_guarded_database_commands_parse_without_http_concepts() {
        let query = parse_query(vec![
            "RETURN 1".to_owned(),
            "--server".to_owned(),
            "nostdb://127.0.0.1:7878".to_owned(),
            "--database".to_owned(),
            "knowledge".to_owned(),
            "--credential-file".to_owned(),
            "client.token".to_owned(),
            "--read-only".to_owned(),
        ])
        .expect("remote query parses");
        assert!(query.read_only);
        assert_eq!(
            query.remote.expect("remote options exist").server,
            "nostdb://127.0.0.1:7878"
        );
        assert_eq!(query.common.database, PathBuf::from("knowledge"));

        let drop = parse_database(vec![
            "drop".to_owned(),
            "knowledge".to_owned(),
            "--confirm".to_owned(),
            "knowledge".to_owned(),
            "--server".to_owned(),
            "nostdb://127.0.0.1:7878".to_owned(),
        ])
        .expect("guarded drop parses");
        assert!(matches!(
            drop.command,
            DatabaseCommand::Drop { ref name, ref confirm_name }
                if name == "knowledge" && confirm_name == "knowledge"
        ));
        assert!(
            parse_database(vec![
                "drop".to_owned(),
                "knowledge".to_owned(),
                "--server".to_owned(),
                "nostdb://127.0.0.1:7878".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn remote_wire_results_use_the_existing_machine_formats() {
        let statement = serde_json::json!({
            "kind": "read",
            "result": {
                "columns": ["value"],
                "rows": [[1], [2]],
                "ordered": true
            }
        });
        let mut output = Vec::new();
        render_wire_statement(&statement, OutputFormat::Jsonl, &mut output)
            .expect("wire result renders");
        assert_eq!(
            String::from_utf8(output).expect("UTF-8"),
            "{\"value\":1}\n{\"value\":2}\n"
        );
    }
}
