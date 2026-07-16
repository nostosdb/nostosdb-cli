#![forbid(unsafe_code)]

const HELP: &str = "NostosDB CLI placeholder

Usage:
    nostos --help
    nostos --version

Only help and version are available in Stage 0.
CLI functionality is deferred to Stage 7.";

fn main() {
    let mut arguments = std::env::args().skip(1);
    let first = arguments.next();
    let has_extra = arguments.next().is_some();

    if has_extra {
        unsupported();
    }

    match first.as_deref() {
        None | Some("-h" | "--help") => println!("{HELP}"),
        Some("-V" | "--version") => println!("nostos {}", env!("CARGO_PKG_VERSION")),
        Some(_) => unsupported(),
    }
}

fn unsupported() -> ! {
    eprintln!("nostos: only --help and --version are available in Stage 0");
    std::process::exit(2);
}
