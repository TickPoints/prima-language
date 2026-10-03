//! `prima new` / `prima init` project scaffolding (spec §20 project layout).
//!
//! Both commands write the same skeleton: the `src/main.pra` entry, a `prima.toml` manifest,
//! a `config.toml` placeholder, a README, and a `.gitignore` that ignores `outputs/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};

use crate::project::{ENTRY_REL, MANIFEST};

/// `prima new <name>`: create a new project directory. Fails if the target exists and is not
/// empty, so an accidental overwrite is never silent.
pub fn new_project(name: &str) -> anyhow::Result<ExitCode> {
    if name.trim().is_empty() {
        bail!("project name must not be empty");
    }
    let dir = PathBuf::from(name);
    if dir.exists() {
        let empty = fs::read_dir(&dir)
            .with_context(|| format!("cannot read {}", dir.display()))?
            .next()
            .is_none();
        if !empty {
            bail!(
                "`{}` already exists and is not empty; choose another name or run `prima init` inside it",
                dir.display()
            );
        }
    }
    let package = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string();
    scaffold(&dir, &package)?;
    println!("Created Prima project `{package}` in {}", dir.display());
    println!("  cd {} && prima run", dir.display());
    Ok(ExitCode::SUCCESS)
}

/// `prima init`: scaffold the current directory. Fails when it is already a Prima project.
pub fn init() -> anyhow::Result<ExitCode> {
    let cwd = std::env::current_dir().context("cannot determine the current directory")?;
    if cwd.join(MANIFEST).exists() || cwd.join(ENTRY_REL).exists() {
        bail!(
            "`{}` is already a Prima project (`{MANIFEST}` or `{ENTRY_REL}` exists)",
            cwd.display()
        );
    }
    let package = cwd
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("app")
        .to_string();
    scaffold(&cwd, &package)?;
    println!("Initialized Prima project `{package}` in {}", cwd.display());
    println!("  prima run");
    Ok(ExitCode::SUCCESS)
}

/// Write the project skeleton under `root`. Existing files are left untouched (idempotent for
/// `init`-style use on an empty tree; `new`/`init` already guard the common cases).
fn scaffold(root: &Path, package: &str) -> anyhow::Result<()> {
    let src = root.join("src");
    fs::create_dir_all(&src).with_context(|| format!("cannot create {}", src.display()))?;

    write_new(&src.join("main.pra"), &entry_source(package))?;
    write_new(&root.join(MANIFEST), &manifest(package))?;
    write_new(&root.join("config.toml"), CONFIG)?;
    write_new(&root.join("README.md"), &readme(package))?;
    write_new(&root.join(".gitignore"), GITIGNORE)?;
    Ok(())
}

fn write_new(path: &Path, contents: &str) -> anyhow::Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, contents).with_context(|| format!("cannot write {}", path.display()))
}

fn entry_source(package: &str) -> String {
    format!(
        "//! {package} — a Prima project (spec §20).\n\
         config {{\n\
         \x20   fraction := true\n\
         \x20   broadcast := true\n\
         }}\n\
         \n\
         println(\"hello from {package}\");\n"
    )
}

fn manifest(package: &str) -> String {
    format!(
        "[package]\n\
         name = \"{package}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2.3\"\n"
    )
}

const CONFIG: &str = "# Project-level toolchain configuration (spec §20).\n\
                      # Compiler targets, default optimization channels, and tool arguments live here.\n";

const GITIGNORE: &str = "/outputs/\n";

fn readme(package: &str) -> String {
    format!(
        "# {package}\n\
         \n\
         A Prima project.\n\
         \n\
         ## Layout\n\
         \n\
         - `src/main.pra` — project entry / root module\n\
         - `{MANIFEST}` — project metadata\n\
         - `config.toml` — project-level toolchain configuration\n\
         - `outputs/` — run artifacts (git-ignored)\n\
         \n\
         ## Commands\n\
         \n\
         ```bash\n\
         prima run      # run src/main.pra\n\
         prima check    # static checks\n\
         prima test     # run every *.pra under src/\n\
         prima fmt -w   # format source\n\
         ```\n"
    )
}
