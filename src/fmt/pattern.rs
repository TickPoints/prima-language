//! Pattern-source rendering for `prima fmt` (spec §4.4): bindings, wildcards, literals,
//! tuple/array destructuring (with `..` rest), struct and variant patterns, ranges, `|`-or and
//! grouped patterns. Shared with `prima doc`, so `format_pattern` is `pub(crate)` and
//! re-exported at `crate::fmt`.

use prima_syntax::ast::Pattern;

use super::text::format_literal;

pub(crate) fn format_pattern(pat: &Pattern, out: &mut String) {
    match pat {
        Pattern::Wildcard(_) => out.push('_'),
        Pattern::Binding(name) => out.push_str(&name.value),
        Pattern::Literal(lit) => format_literal(lit, out),
        Pattern::Tuple(pats, rest) => {
            out.push('(');
            format_pattern_list(pats, rest, out);
            out.push(')');
        }
        Pattern::Array(pats, rest) => {
            out.push('[');
            format_pattern_list(pats, rest, out);
            out.push(']');
        }
        Pattern::Struct { name, fields, rest } => {
            out.push_str(&name.value);
            out.push_str(" { ");
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&f.name.value);
                if let Some(p) = &f.pat {
                    out.push_str(": ");
                    format_pattern(p, out);
                }
            }
            if *rest {
                if !fields.is_empty() {
                    out.push_str(", ");
                }
                out.push_str("..");
            }
            out.push_str(" }");
        }
        Pattern::Variant { name, args, .. } => {
            out.push_str(&name.value);
            if !args.is_empty() {
                out.push('(');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    format_pattern(a, out);
                }
                out.push(')');
            }
        }
        Pattern::Range { lo, hi, inclusive } => {
            format_literal(lo, out);
            if *inclusive {
                out.push_str("..=");
            } else {
                out.push_str("..");
            }
            format_literal(hi, out);
        }
        Pattern::Or(pats) => {
            for (i, p) in pats.iter().enumerate() {
                if i > 0 {
                    out.push_str(" | ");
                }
                format_pattern(p, out);
            }
        }
        Pattern::Group(p) => {
            out.push('(');
            format_pattern(p, out);
            out.push(')');
        }
    }
}

fn format_pattern_list(pats: &[Pattern], rest: &bool, out: &mut String) {
    for (i, p) in pats.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        format_pattern(p, out);
    }
    if *rest {
        if !pats.is_empty() {
            out.push_str(", ");
        }
        out.push_str("..");
    }
}
