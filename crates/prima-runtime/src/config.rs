//! Policy system (spec §13): configuration values for the three-level policy and the merge basis.
//!
//! Precedence **local > module > global** (spec §13.1): the evaluator holds a stack of
//! `Config` values; `with config` pushes and leaving a block pops (spec §4.6); this module applies
//! the AST values of `config {}` entries to a `Config`.

use prima_syntax::ast::{ConfigEntry, Expr, ExprKind, Literal};

use crate::error::RuntimeError;

/// Domain annotation (spec §6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Real,
    Complex,
    Integer,
    Positive,
    NonNegative,
    NonZero,
}

/// `undefined_handling` policy (spec §13.2): `strict` errors / `custom { 0/0 := 1 }` black magic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndefinedHandling {
    Strict,
    Custom,
}

/// Print format (spec §13.2); currently only the `latex` renderer is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintFormat {
    Latex,
    Unicode,
    Ascii,
}

/// Operator-overload usage policy (spec §13.2/§18.5): `warn` (default, emits `W0005`),
/// `allow` (no warning), or `deny` (using an overload is an error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverloadPolicy {
    Warn,
    Allow,
    Deny,
}

/// Optimization level (spec §10.2/§13.2): an incrementally-configured set of compiler optimization
/// channels. `O0 < O1 < O2 < O3`; each tier enables all channels of the lower tiers. Default `O2`.
/// Distinct from `simplify_level` (symbolic-semantic, spec §8.3): `opt_level` only controls
/// compiler-applied optimizations and never changes observable program semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OptLevel {
    O0,
    O1,
    O2,
    O3,
}

impl OptLevel {
    /// The tier as a `u8` (O0=0 .. O3=3), for comparing against an `@builtin(ON)` annotation.
    pub const fn tier(self) -> u8 {
        match self {
            OptLevel::O0 => 0,
            OptLevel::O1 => 1,
            OptLevel::O2 => 2,
            OptLevel::O3 => 3,
        }
    }

    /// Convert a validated tier number back to an `OptLevel` (clamping out-of-range to `O3`).
    pub const fn from_tier(n: u8) -> OptLevel {
        match n {
            0 => OptLevel::O0,
            1 => OptLevel::O1,
            2 => OptLevel::O2,
            _ => OptLevel::O3,
        }
    }
}

/// Policy configuration (spec §13.2 finalized): `domain`/`undefined_handling` are global (polluting) policies,
/// the rest are module/local policies; defaults match the spec (`fraction`/`broadcast`/`loop_optimization` on by default).
#[derive(Debug, Clone)]
pub struct Config {
    pub domain: Domain,
    pub undefined_handling: UndefinedHandling,
    /// Entries of `custom { 0/0 := 1, ... }` (spec §13.4): pattern → value expression pairs.
    pub custom_rules: Vec<(Expr, Expr)>,
    pub fraction: bool,
    pub broadcast: bool,
    pub loop_optimization: bool,
    pub simplify_level: u8,
    pub opt_level: OptLevel,
    pub num_to_big: bool,
    pub print_format: PrintFormat,
    pub overload_policy: OverloadPolicy,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            domain: Domain::Complex,
            undefined_handling: UndefinedHandling::Strict,
            custom_rules: Vec::new(),
            fraction: true,
            broadcast: true,
            loop_optimization: true,
            simplify_level: 2,
            opt_level: OptLevel::O2,
            num_to_big: true,
            print_format: PrintFormat::Latex,
            overload_policy: OverloadPolicy::Warn,
        }
    }
}

impl Config {
    /// Apply the entries of `config {}` to the current configuration (spec §13.2 policy table).
    pub fn apply(&mut self, entries: &[ConfigEntry]) -> Result<(), RuntimeError> {
        for e in entries {
            match e.name.value.as_str() {
                "domain" => {
                    let v = parse_enum(&e.value, "domain")?;
                    self.domain = match v.as_str() {
                        "real" => Domain::Real,
                        "complex" => Domain::Complex,
                        "integer" => Domain::Integer,
                        "positive" => Domain::Positive,
                        "nonnegative" => Domain::NonNegative,
                        "nonzero" => Domain::NonZero,
                        _ => return Err(RuntimeError::Message(format!("unknown `domain` value `{v}`"))),
                    };
                }
                "undefined_handling" => {
                    match &e.value.kind {
                        ExprKind::Path { segments } if segments.len() == 1 && segments[0].value == "strict" => {
                            self.undefined_handling = UndefinedHandling::Strict;
                        }
                        ExprKind::Custom(items) => {
                            self.undefined_handling = UndefinedHandling::Custom;
                            self.custom_rules = items.clone();
                        }
                        _ => return Err(RuntimeError::Message("invalid value for `undefined_handling`".into())),
                    }
                }
                "fraction" => self.fraction = parse_bool(&e.value, "fraction")?,
                "broadcast" => self.broadcast = parse_bool(&e.value, "broadcast")?,
                "loop_optimization" => self.loop_optimization = parse_bool(&e.value, "loop_optimization")?,
                "simplify_level" => self.simplify_level = parse_int(&e.value, "simplify_level")?,
                "opt_level" => self.opt_level = parse_opt_level(&e.value)?,
                "num_to_big" => self.num_to_big = parse_bool(&e.value, "num_to_big")?,
                "print_format" => {
                    let v = parse_enum(&e.value, "print_format")?;
                    self.print_format = match v.as_str() {
                        "latex" => PrintFormat::Latex,
                        "unicode" => PrintFormat::Unicode,
                        "ascii" => PrintFormat::Ascii,
                        _ => return Err(RuntimeError::Message(format!("unknown `print_format` value `{v}`"))),
                    };
                }
                "overload_policy" => {
                    let v = parse_enum(&e.value, "overload_policy")?;
                    self.overload_policy = match v.as_str() {
                        "warn" => OverloadPolicy::Warn,
                        "allow" => OverloadPolicy::Allow,
                        "deny" => OverloadPolicy::Deny,
                        _ => return Err(RuntimeError::Message(format!("unknown `overload_policy` value `{v}`"))),
                    };
                }
                other => return Err(RuntimeError::Message(format!("unknown config key `{other}`"))),
            }
        }
        Ok(())
    }
}

fn parse_bool(e: &Expr, key: &str) -> Result<bool, RuntimeError> {
    match &e.kind {
        ExprKind::Literal(Literal::Bool(b)) => Ok(*b),
        _ => Err(RuntimeError::Message(format!("config `{key}` expects a bool"))),
    }
}

fn parse_int(e: &Expr, key: &str) -> Result<u8, RuntimeError> {
    match &e.kind {
        ExprKind::Literal(Literal::Integer(s)) => s
            .parse::<u8>()
            .map_err(|_| RuntimeError::Message(format!("config `{key}` expects an integer 0..=3"))),
        _ => Err(RuntimeError::Message(format!("config `{key}` expects an integer"))),
    }
}

fn parse_enum(e: &Expr, key: &str) -> Result<String, RuntimeError> {
    match &e.kind {
        ExprKind::Path { segments } if segments.len() == 1 => Ok(segments[0].value.clone()),
        ExprKind::Symbol(s) => Ok(s.value.clone()),
        _ => Err(RuntimeError::Message(format!("config `{key}` expects an enum value"))),
    }
}

/// Parse an `opt_level` config value (spec §10.2/§13.2), accepting the `O0`..`O3` enum forms.
fn parse_opt_level(e: &Expr) -> Result<OptLevel, RuntimeError> {
    let v = parse_enum(e, "opt_level")?;
    match v.as_str() {
        "O0" => Ok(OptLevel::O0),
        "O1" => Ok(OptLevel::O1),
        "O2" => Ok(OptLevel::O2),
        "O3" => Ok(OptLevel::O3),
        _ => Err(RuntimeError::Message(format!("unknown `opt_level` value `{v}` (expected O0..=O3)"))),
    }
}
