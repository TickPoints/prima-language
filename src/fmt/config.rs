//! Config-block source rendering for `prima fmt` (spec §13.1): the top-level `config { ... }`
//! block and each of its entries, emitted in the canonical `fraction := true` / `fraction: bool = true`
//! forms depending on whether the entry carries a type annotation.

use prima_syntax::ast::{ConfigBlock, ConfigEntry};

use super::expr::format_expr;
use super::ty::format_type;

pub(crate) fn format_config_block(cfg: &ConfigBlock, out: &mut String) {
    out.push_str("config { ");
    for (i, entry) in cfg.entries.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        format_config_entry(entry, out);
    }
    out.push_str(" }");
}

pub(crate) fn format_config_entry(entry: &ConfigEntry, out: &mut String) {
    out.push_str(&entry.name.value);
    match &entry.type_ann {
        // `fraction: bool = true` (appendix BNF) when annotated, `fraction := true` otherwise (spec §13.1).
        Some(t) => {
            out.push_str(": ");
            format_type(t, out);
            out.push_str(" = ");
        }
        None => out.push_str(" := "),
    }
    format_expr(&entry.value, 0, out);
}
