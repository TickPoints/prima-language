use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use prima_runtime::check::check_src;
use prima_runtime::Evaluator;

mod diagnostics;

/// Prima toolchain CLI (spec §20): `run`/`parse`/`check` are available; the remaining subcommands are placeholders.
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
    Compile { file: PathBuf, #[arg(short, long)] output: Option<PathBuf> },
    Repl,
    Fmt { path: PathBuf },
    Check { file: PathBuf },
    Test,
    Doc,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { file } => run_file(&file),
        Command::Parse { file } => parse_file(&file),
        Command::Check { file } => check_file(&file),
        _ => {
            eprintln!("not implemented yet");
            ExitCode::FAILURE
        }
    }
}

fn read_src(file: &Path) -> Result<String, ExitCode> {
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

// Static check (spec §16.2/§16.4): collect syntax and statically detectable type errors without executing.
fn check_file(file: &Path) -> ExitCode {
    let source = match read_src(file) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let errors = check_src(&source);
    if errors.is_empty() {
        return ExitCode::SUCCESS;
    }
    diagnostics::report_type_errors(file, &source, &errors);
    ExitCode::FAILURE
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
