use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use prima_runtime::Evaluator;
use prima_runtime::check::check_src_checked;
use prima_syntax::parse_checked;

mod cabi;
mod completions;
mod diagnostics;
mod doc;
mod doctest;
mod fmt;
mod newcmd;
mod project;
mod repl;
mod testcmd;

use diagnostics::{ColorMode, RenderOptions};

/// `--color` policy (spec §20); auto-detect a terminal by default.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum ColorArg {
    #[default]
    Auto,
    Always,
    Never,
}

impl From<ColorArg> for ColorMode {
    fn from(value: ColorArg) -> Self {
        match value {
            ColorArg::Auto => ColorMode::Auto,
            ColorArg::Always => ColorMode::Always,
            ColorArg::Never => ColorMode::Never,
        }
    }
}

/// Prima toolchain CLI (spec §20): `run`/`parse`/`compile`/`check`/`repl`/`fmt`/`test`/`doc`.
#[derive(Parser)]
#[command(name = "prima", version, about = "Prima language toolchain")]
pub(crate) struct Cli {
    /// Control colored diagnostic output.
    #[arg(long, global = true, value_enum, default_value_t = ColorArg::Auto)]
    color: ColorArg,
    /// Suppress non-fatal warnings.
    #[arg(long, short, global = true)]
    quiet: bool,
    /// Emit machine-readable NDJSON diagnostics on stderr (rustc `--message-format=json` style).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Interpret a program (the file is the root module). Defaults to the project entry.
    Run { file: Option<PathBuf> },
    /// Dump the AST of a source file.
    Parse { file: PathBuf },
    /// Emit a C header or build a C-ABI shared library.
    Compile {
        file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        emit_headers: bool,
        #[arg(long)]
        emit_c_abi: bool,
    },
    /// Start an interactive session.
    Repl,
    /// Format source. Defaults to the project entry when no path is given.
    Fmt {
        path: Option<PathBuf>,
        #[arg(short, long)]
        write: bool,
        #[arg(long)]
        check: bool,
    },
    /// Statically check a file. Defaults to the project entry when no path is given.
    Check {
        file: Option<PathBuf>,
        /// Promote the given warning codes (e.g. `W0005`) to errors (spec §16.5).
        #[arg(long = "deny")]
        deny: Vec<String>,
    },
    /// Run every `*.pra` file under a directory (default: `src/` in a project, else `examples/`).
    Test { path: Option<PathBuf> },
    /// Generate Markdown docs from `///` comments. Defaults to the project entry.
    Doc {
        /// Source file to document (omitted with `--stdlib`).
        path: Option<PathBuf>,
        /// Write the Markdown to a file instead of stdout (spec §20).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Document the embedded stdlib modules instead of a file (spec §20).
        #[arg(long)]
        stdlib: bool,
        /// Validate `///` doc code blocks: statically check each ```pra block (and run it when
        /// `--run` is given). Reported as a `doc-test` outcome (spec §20).
        #[arg(long)]
        test: bool,
        /// With `--test`: also execute each doc block and compare to its `// expect:` line.
        #[arg(long)]
        run: bool,
    },
    /// Create a new project skeleton in `<name>/` (spec §20).
    New { name: String },
    /// Create a project skeleton in the current directory (spec §20).
    Init,
    /// Generate a shell completion script to stdout.
    Completions { shell: Shell },
}

fn main() -> ExitCode {
    prima_stdlib::init();
    let cli = Cli::parse();
    diagnostics::set_options(RenderOptions {
        color: cli.color.into(),
        json: cli.json,
        quiet: cli.quiet,
    });
    match dispatch(cli) {
        Ok(code) => code,
        Err(e) => report_anyhow(&e),
    }
}

/// Route a parsed CLI command to its handler. Each handler returns an `anyhow::Result<ExitCode>` so
/// non-source errors (I/O, C-ABI build, etc.) carry a contextual `source` chain, while the textual
/// diagnostics renderer (`diagnostics::*`) still owns source-level (syntax/type/runtime) output.
fn dispatch(cli: Cli) -> Result<ExitCode> {
    let cwd = std::env::current_dir().context("cannot determine the current directory")?;
    match cli.command {
        Command::Run { file } => run_file(&project::resolve_entry(file.as_deref(), &cwd)?),
        Command::Parse { file } => parse_file(&file),
        Command::Check { file, deny } => {
            check_file(&project::resolve_entry(file.as_deref(), &cwd)?, &deny)
        }
        // `--emit-c-abi` also writes the header, so it takes precedence when both flags are set.
        Command::Compile {
            file,
            output,
            emit_c_abi: true,
            ..
        } => cabi::run(&file, output.as_deref()),
        Command::Compile {
            file,
            output,
            emit_headers: true,
            ..
        } => compile_headers(&file, output.as_deref()),
        Command::Compile { .. } => {
            diagnostics::print_colored_error(
                "compilation requires `--emit-headers` or `--emit-c-abi` in this build (spec §20)",
            );
            Ok(ExitCode::FAILURE)
        }
        Command::Repl => repl::run(),
        Command::Fmt { path, write, check } => {
            let path = project::resolve_entry(path.as_deref(), &cwd)?;
            fmt::run(&path, write, check)
        }
        Command::Test { path } => {
            let dir = path.unwrap_or_else(|| project::resolve_test_dir(&cwd));
            testcmd::run(&dir)
        }
        Command::Doc {
            path,
            output,
            stdlib,
            test,
            run,
        } => {
            let path = if stdlib || path.is_some() {
                path
            } else {
                Some(project::resolve_entry(None, &cwd)?)
            };
            doc::run(path.as_deref(), output.as_deref(), stdlib, test, run)
        }
        Command::New { name } => newcmd::new_project(&name),
        Command::Init => newcmd::init(),
        Command::Completions { shell } => completions::run(shell),
    }
}

/// Render a top-level `anyhow` error as a brief `error:` line and its source chain as `caused by:`
/// lines, so the CLI keeps a single, predictable format for non-source failures.
fn report_anyhow(err: &anyhow::Error) -> ExitCode {
    diagnostics::print_colored_error(&format!("{err}"));
    for cause in err.chain().skip(1) {
        eprintln!("caused by: {cause}");
    }
    ExitCode::FAILURE
}

pub(crate) fn read_src(file: &Path) -> Result<String> {
    std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))
}

// Interpreted execution (spec §20): the file is the root module; parse + module system + evaluation.
fn run_file(file: &Path) -> Result<ExitCode> {
    let source = read_src(file)?;
    // Root-file syntax errors render as rustc-style diagnostics (spec §16.4).
    if let Err(errors) = prima_syntax::parse(&source) {
        diagnostics::report_syntax_errors(file, &source, &errors);
        return Ok(ExitCode::FAILURE);
    }
    let mut ev = Evaluator::new();
    match ev.eval_file(file) {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(e) => {
            diagnostics::report_runtime_error(file, &source, &e);
            Ok(ExitCode::FAILURE)
        }
    }
}

// Static check (spec §16.2/§16.4/§16.5): collect syntax and statically detectable type errors
// without executing. All parse warnings are rendered; a warning whose code is in the `--deny`
// set is promoted to an error and makes the check fail.
fn check_file(file: &Path, deny: &[String]) -> Result<ExitCode> {
    let source = read_src(file)?;
    let (_, syntax_errors, parse_warnings) = parse_checked(&source);
    if !syntax_errors.is_empty() {
        diagnostics::report_syntax_errors(file, &source, &syntax_errors);
        return Ok(ExitCode::FAILURE);
    }

    // Static check (spec §6.3/§16.2): type errors plus compiler-collected warnings (e.g. `W0003`
    // unused binding). Parse-time warnings from `parse_checked` are merged with these.
    let (errors, check_warnings) = check_src_checked(&source);
    // The visitor warnings may carry zero-width spans; filter those that overlap the file.
    let mut warnings = parse_warnings;
    warnings.extend(check_warnings);

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
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

fn parse_file(file: &Path) -> Result<ExitCode> {
    let source = read_src(file)?;
    match prima_syntax::parse(&source) {
        Ok(program) => {
            println!("{program:#?}");
            Ok(ExitCode::SUCCESS)
        }
        Err(errors) => {
            diagnostics::report_syntax_errors(file, &source, &errors);
            Ok(ExitCode::FAILURE)
        }
    }
}

// C header emission for `@c_api::extern` exports (spec §18.4): parse, collect the C-ABI prototype
// list, and render the include-guarded header to `--output` (or stdout when no path is given).
fn compile_headers(file: &Path, output: Option<&Path>) -> Result<ExitCode> {
    let source = read_src(file)?;
    let program = match prima_syntax::parse(&source) {
        Ok(p) => p,
        Err(errors) => {
            diagnostics::report_syntax_errors(file, &source, &errors);
            return Ok(ExitCode::FAILURE);
        }
    };
    let header =
        prima_runtime::capi::render_header(&prima_runtime::capi::collect_exports(&program));
    match output {
        Some(path) => {
            std::fs::write(path, &header)
                .with_context(|| format!("cannot write {}", path.display()))?;
            Ok(ExitCode::SUCCESS)
        }
        None => {
            print!("{header}");
            Ok(ExitCode::SUCCESS)
        }
    }
}
