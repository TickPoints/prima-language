//! `prima completions <shell>` (spec §20): generate a shell completion script to stdout.
//!
//! Uses `clap_complete` against the live CLI definition, so completions always match the
//! accepted subcommands and flags.

use std::process::ExitCode;

use clap::CommandFactory;
use clap_complete::{Shell, generate};

/// Write the completion script for `shell` to stdout.
pub fn run(shell: Shell) -> anyhow::Result<ExitCode> {
    let mut command = crate::Cli::command();
    let bin_name = command.get_name().to_string();
    generate(shell, &mut command, bin_name, &mut std::io::stdout());
    Ok(ExitCode::SUCCESS)
}
