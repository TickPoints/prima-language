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
    Compile {
        file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        emit_headers: bool,
    },
    Repl,
    Fmt { path: PathBuf },
    Check { file: PathBuf },
    Test,
    Doc,
}

fn main() -> ExitCode {
    prima_stdlib::init();
    let cli = Cli::parse();
    match cli.command {
        Command::Run { file } => run_file(&file),
        Command::Parse { file } => parse_file(&file),
        Command::Check { file } => check_file(&file),
        Command::Compile { file, output, emit_headers: true } => compile_headers(&file, output.as_deref()),
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
