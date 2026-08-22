use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use prima_runtime::check::check_src;
use prima_runtime::Evaluator;
use prima_syntax::parse_checked;

mod cabi;
mod diagnostics;
mod doc;
mod fmt;
mod repl;
mod testcmd;

/// Prima toolchain CLI (spec §20): `run`/`parse`/`compile`/`check`/`repl`/`fmt`/`test`/`doc`.
#[derive(Parser)]
#[command(name = "prima", version, about = "Prima language toolchain")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Run { file: PathBuf },
    Parse { file: PathBuf },
    Compile {
        file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        emit_headers: bool,
        #[arg(long)]
        emit_c_abi: bool,
    },
    Repl,
    Fmt {
        path: PathBuf,
        #[arg(short, long)]
        write: bool,
        #[arg(long)]
        check: bool,
    },
    Check {
        file: PathBuf,
        /// Promote the given warning codes (e.g. `W0005`) to errors (spec §16.5).
        #[arg(long = "deny")]
        deny: Vec<String>,
    },
    Test {
        path: Option<PathBuf>,
    },
    Doc {
        /// Source file to document (omitted with `--stdlib`).
        path: Option<PathBuf>,
        /// Write the Markdown to a file instead of stdout (spec §20).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Document the embedded stdlib modules instead of a file (spec §20).
        #[arg(long)]
        stdlib: bool,
    },
}

fn main() -> ExitCode {
    prima_stdlib::init();
    let cli = Cli::parse();
    match cli.command {
        Command::Run { file } => run_file(&file),
        Command::Parse { file } => parse_file(&file),
        Command::Check { file, deny } => check_file(&file, &deny),
        // `--emit-c-abi` also writes the header, so it takes precedence when both flags are set.
        Command::Compile { file, output, emit_c_abi: true, .. } => cabi::run(&file, output.as_deref()),
        Command::Compile { file, output, emit_headers: true, .. } => compile_headers(&file, output.as_deref()),
        Command::Compile { .. } => {
            diagnostics::print_colored_error("compilation requires `--emit-headers` or `--emit-c-abi` in this build (spec §20)");
            ExitCode::FAILURE
        }
        Command::Repl => repl::run(),
        Command::Fmt { path, write, check } => fmt::run(&path, write, check),
        Command::Test { path } => testcmd::run(&path.unwrap_or_else(|| PathBuf::from(testcmd::DEFAULT_DIR))),
        Command::Doc { path, output, stdlib } => doc::run(path.as_deref(), output.as_deref(), stdlib),
    }
}

pub(crate) fn read_src(file: &Path) -> Result<String, ExitCode> {
    match std::fs::read_to_string(file) {
        Ok(s) => Ok(s),
        Err(e) => {
            diagnostics::print_colored_error(&format!("cannot read {}: {e}", file.display()));
            Err(ExitCode::FAILURE)
        }
    }
}

// Interpreted execution (spec §20): the file is the root module; parse + module system + evaluation.
fn run_file(file: &Path) -> ExitCode {
    let source = match read_src(file) {
        Ok(s) => s,
        Err(code) => return code,
    };
    // Root-file syntax errors render as rustc-style diagnostics (spec §16.4).
    if let Err(errors) = prima_syntax::parse(&source) {
        diagnostics::report_syntax_errors(file, &source, &errors);
        return ExitCode::FAILURE;
    }
    let mut ev = Evaluator::new();
    match ev.eval_file(file) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            diagnostics::report_runtime_error(file, &source, &e);
            ExitCode::FAILURE
        }
    }
}

// Static check (spec §16.2/§16.4/§16.5): collect syntax and statically detectable type errors
// without executing. All parse warnings are rendered; a warning whose code is in the `--deny`
// set is promoted to an error and makes the check fail.
fn check_file(file: &Path, deny: &[String]) -> ExitCode {
    let source = match read_src(file) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let (_, syntax_errors, warnings) = parse_checked(&source);
    if !syntax_errors.is_empty() {
        diagnostics::report_syntax_errors(file, &source, &syntax_errors);
        return ExitCode::FAILURE;
    }

    let errors = check_src(&source);
    let denied: Vec<_> = warnings
        .iter()
        .filter(|w| deny.iter().any(|d| d == w.code))
        .cloned()
        .collect();
    let allowed: Vec<_> = warnings
        .iter()
        .filter(|w| !deny.iter().any(|d| d == w.code))
        .cloned()
        .collect();

    if !allowed.is_empty() {
        diagnostics::report_warnings(file, &source, &allowed);
    }
    if !denied.is_empty() {
        diagnostics::report_denied_warnings(file, &source, &denied);
    }
    if !errors.is_empty() {
        diagnostics::report_type_errors(file, &source, &errors);
    }

    if errors.is_empty() && denied.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn parse_file(file: &Path) -> ExitCode {
    let source = match read_src(file) {
        Ok(s) => s,
        Err(code) => return code,
    };
    match prima_syntax::parse(&source) {
        Ok(program) => {
            println!("{program:#?}");
            ExitCode::SUCCESS
        }
        Err(errors) => {
            diagnostics::report_syntax_errors(file, &source, &errors);
            ExitCode::FAILURE
        }
    }
}

// C header emission for `@c_api::extern` exports (spec §18.4): parse, collect the C-ABI prototype
// list, and render the include-guarded header to `--output` (or stdout when no path is given).
fn compile_headers(file: &Path, output: Option<&Path>) -> ExitCode {
    let source = match read_src(file) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let program = match prima_syntax::parse(&source) {
        Ok(p) => p,
        Err(errors) => {
            diagnostics::report_syntax_errors(file, &source, &errors);
            return ExitCode::FAILURE;
        }
    };
    let header = prima_runtime::capi::render_header(&prima_runtime::capi::collect_exports(&program));
    match output {
        Some(path) => match std::fs::write(path, &header) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                diagnostics::print_colored_error(&format!("cannot write {}: {e}", path.display()));
                ExitCode::FAILURE
            }
        },
        None => {
            print!("{header}");
            ExitCode::SUCCESS
        }
    }
}
