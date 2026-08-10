use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use prima_runtime::Evaluator;

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
        _ => {
            eprintln!("not implemented yet");
            ExitCode::FAILURE
        }
    }
}

fn read_src(file: &PathBuf) -> Result<String, ExitCode> {
    match std::fs::read_to_string(file) {
        Ok(s) => Ok(s),
        Err(e) => {
            eprintln!("error reading {}: {e}", file.display());
            Err(ExitCode::FAILURE)
        }
    }
}

fn run_file(file: &PathBuf) -> ExitCode {
    let src = match read_src(file) {
        Ok(s) => s,
        Err(code) => return code,
    };
    match prima_syntax::parse(&src) {
        Ok(program) => {
            let mut ev = Evaluator::new();
            match ev.eval_program(&program) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(errors) => {
            for e in &errors {
                eprintln!("error at {}..{}: {}", e.span.start, e.span.end, e.message);
            }
            ExitCode::FAILURE
        }
    }
}

fn parse_file(file: &PathBuf) -> ExitCode {
    let src = match read_src(file) {
        Ok(s) => s,
        Err(code) => return code,
    };
    match prima_syntax::parse(&src) {
        Ok(program) => {
            println!("{program:#?}");
            ExitCode::SUCCESS
        }
        Err(errors) => {
            for e in &errors {
                eprintln!("error at {}..{}: {}", e.span.start, e.span.end, e.message);
            }
            ExitCode::FAILURE
        }
    }
}
