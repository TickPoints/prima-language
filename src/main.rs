use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "prima", version, about = "Prima language toolchain")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Run { file: PathBuf },
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
        _ => {
            eprintln!("not implemented yet");
            ExitCode::FAILURE
        }
    }
}

fn run_file(file: &PathBuf) -> ExitCode {
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
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
