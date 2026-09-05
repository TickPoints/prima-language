//! Expression-source rendering for `prima fmt` (spec §4.3/§18.1): literals, f-strings, symbol/path
//! references, calls/methods/fields/indexing, binary and unary operators, collections and
//! comprehensions, lambdas, and match expressions. Parentheses are re-inserted from the Pratt
//! binding-power table (`binop_lbp`/`binop_rbp`/`root_prec`, spec appendix A) when the AST would
//! otherwise re-parse to a different tree.

use prima_syntax::ast::{
    BinOp, CompKind, ComprehensionClause, Expr, ExprKind, FStringPart, Index, IndexItem, MatchArm,
    UnOp,
};

use super::import::format_path;
use super::pattern::format_pattern;
use super::text::{escape_string, format_literal, push_indent};
use super::ty::format_params_bare;

/// Unary binding power from the parser (`UNARY_BP`, spec §4.3): unary binds looser than `^`.
const UNARY_BP: u8 = 7;
/// Precedence of atoms and postfix chains (calls, index, field, method, literals): tighter than any operator.
pub(crate) const ATOM_BP: u8 = 100;

/// Binding power (lbp) of a binary operator when embedded as a subexpression (parser `binop_bp`).
fn binop_lbp(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 2,
        BinOp::And => 3,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In => 4,
        BinOp::Add | BinOp::Sub | BinOp::Union | BinOp::Difference => 5,
        BinOp::Mul
        | BinOp::Div
        | BinOp::Mod
        | BinOp::MatMul
        | BinOp::Broadcast
        | BinOp::Intersect => 6,
        BinOp::Pow => 8,
    }
}

/// Right binding power (rbp) of a binary operator for its right operand (parser `binop_bp`).
fn binop_rbp(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 3,
        BinOp::And => 4,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In => 5,
        BinOp::Add | BinOp::Sub | BinOp::Union | BinOp::Difference => 6,
        BinOp::Mul
        | BinOp::Div
        | BinOp::Mod
        | BinOp::MatMul
        | BinOp::Broadcast
        | BinOp::Intersect => 7,
        BinOp::Pow => 8,
    }
}

fn binop_sym(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Pow => "^",
        BinOp::MatMul => "@",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::In => "in",
        BinOp::Union => "∪",
        BinOp::Intersect => "∩",
        BinOp::Difference => "\\",
        BinOp::Broadcast => "@.",
    }
}

fn unary_sym(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
        UnOp::Pos => "+",
    }
}

/// Precedence at which an expression binds when embedded as an operand (for paren re-insertion).
fn root_prec(e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::Binary { op, .. } => binop_lbp(*op),
        ExprKind::Unary { .. } => UNARY_BP,
        _ => ATOM_BP,
    }
}

pub(crate) fn format_expr(e: &Expr, min_bp: u8, out: &mut String) {
    if root_prec(e) < min_bp {
        out.push('(');
        format_expr(e, 0, out);
        out.push(')');
        return;
    }
    match &e.kind {
        ExprKind::Literal(lit) => format_literal(lit, out),
        ExprKind::FString(parts) => format_fstring(parts, out),
        ExprKind::Symbol(name) => out.push_str(&name.value),
        ExprKind::Path { segments } => format_path(segments, out),
        ExprKind::Self_ => out.push_str("self"),
        ExprKind::Call { callee, args } => {
            format_expr(callee, ATOM_BP, out);
            format_args(args, out);
        }
        ExprKind::MethodCall {
            receiver,
            name,
            args,
        } => {
            format_expr(receiver, ATOM_BP, out);
            out.push('.');
            out.push_str(&name.value);
            format_args(args, out);
        }
        ExprKind::Field { receiver, name } => {
            format_expr(receiver, ATOM_BP, out);
            out.push('.');
            out.push_str(&name.value);
        }
        ExprKind::StructLiteral { name, fields, base } => {
            out.push_str(&name.value);
            out.push_str(" { ");
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&f.name.value);
                if let Some(v) = &f.value {
                    out.push_str(": ");
                    format_expr(v, 0, out);
                }
            }
            if let Some(base) = base {
                if !fields.is_empty() {
                    out.push_str(", ");
                }
                out.push_str("..");
                format_expr(base, 0, out);
            }
            out.push_str(" }");
        }
        ExprKind::Index { base, index } => {
            format_expr(base, ATOM_BP, out);
            format_index(index, out);
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let lbp = binop_lbp(*op);
            // `^` is right-associative (spec §4.3): a same-precedence left operand must be parenthesized.
            let lhs_min = if *op == BinOp::Pow { lbp + 1 } else { lbp };
            format_expr(lhs, lhs_min, out);
            if *op == BinOp::Pow {
                // Exponent notation is tight (`x^2`, spec §4.3), matching the canonical examples.
                out.push_str(binop_sym(*op));
            } else {
                out.push(' ');
                out.push_str(binop_sym(*op));
                out.push(' ');
            }
            format_expr(rhs, binop_rbp(*op), out);
        }
        ExprKind::Unary { op, operand } => {
            out.push_str(unary_sym(*op));
            format_expr(operand, UNARY_BP, out);
        }
        ExprKind::Try(operand) => {
            format_expr(operand, ATOM_BP, out);
            out.push('?');
        }
        ExprKind::Array(items) => {
            out.push('[');
            format_expr_list(items, out);
            out.push(']');
        }
        ExprKind::Tuple(items) => {
            out.push('(');
            format_expr_list(items, out);
            out.push(')');
        }
        ExprKind::Dict(pairs) => {
            if pairs.is_empty() {
                // An empty `{}` is an empty Dict (spec §11.6).
                out.push_str("{}");
            } else {
                out.push_str("{ ");
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    format_expr(k, 0, out);
                    out.push_str(": ");
                    format_expr(v, 0, out);
                }
                out.push_str(" }");
            }
        }
        ExprKind::Set(items) => {
            if items.is_empty() {
                out.push_str("{}");
            } else {
                out.push_str("{ ");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    format_expr(item, 0, out);
                }
                out.push_str(" }");
            }
        }
        ExprKind::Comprehension {
            kind,
            output,
            clauses,
        } => {
            format_comp_frame(*kind, out);
            format_expr(output, 0, out);
            for clause in clauses {
                match clause {
                    ComprehensionClause::For { var, iter } => {
                        out.push_str(" for ");
                        out.push_str(&var.value);
                        out.push_str(" in ");
                        format_expr(iter, 0, out);
                    }
                    ComprehensionClause::If { cond } => {
                        out.push_str(" if ");
                        format_expr(cond, 0, out);
                    }
                }
            }
            format_comp_close(*kind, out);
        }
        ExprKind::KeyValue { key, value } => {
            format_expr(key, 0, out);
            out.push_str(": ");
            format_expr(value, 0, out);
        }
        ExprKind::Lambda { params, body } => {
            out.push('|');
            format_params_bare(params, out);
            out.push_str("| ");
            format_expr(body, 0, out);
        }
        ExprKind::Match { scrutinee, arms } => {
            out.push_str("match ");
            format_expr(scrutinee, 0, out);
            out.push_str(" {\n");
            for arm in arms {
                format_match_arm(arm, 1, out);
            }
            out.push('}');
        }
        ExprKind::Custom(pairs) => {
            out.push_str("custom { ");
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_expr(k, 0, out);
                out.push_str(" := ");
                format_expr(v, 0, out);
            }
            out.push_str(" }");
        }
    }
}

fn format_comp_frame(kind: CompKind, out: &mut String) {
    match kind {
        CompKind::Array => out.push('['),
        CompKind::Dict => out.push_str("{ "),
        CompKind::Set => out.push_str("{ "),
        CompKind::Tuple => out.push('('),
    }
}

fn format_comp_close(kind: CompKind, out: &mut String) {
    match kind {
        CompKind::Array => out.push(']'),
        CompKind::Dict | CompKind::Set => out.push_str(" }"),
        CompKind::Tuple => out.push(')'),
    }
}

fn format_args(args: &[Expr], out: &mut String) {
    out.push('(');
    format_expr_list(args, out);
    out.push(')');
}

fn format_expr_list(items: &[Expr], out: &mut String) {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        format_expr(item, 0, out);
    }
}

fn format_index(index: &Index, out: &mut String) {
    out.push('[');
    for (i, item) in index.items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        match item {
            IndexItem::Elem(e) => format_expr(e, 0, out),
            IndexItem::Slice { start, end } => {
                if let Some(s) = start {
                    format_expr(s, 0, out);
                }
                out.push_str("..");
                if let Some(e) = end {
                    format_expr(e, 0, out);
                }
            }
        }
    }
    out.push(']');
}

pub(crate) fn format_match_arm(arm: &MatchArm, indent: usize, out: &mut String) {
    push_indent(indent, out);
    format_pattern(&arm.pattern, out);
    if let Some(guard) = &arm.guard {
        out.push_str(" if ");
        format_expr(guard, 0, out);
    }
    out.push_str(" => ");
    format_expr(&arm.body, 0, out);
    // The parser accepts a trailing `,`/`;` per arm; the canonical form is `,` (spec appendix A `match_arm`).
    out.push_str(",\n");
}

pub(crate) fn format_step(step: &Option<Expr>, out: &mut String) {
    if let Some(s) = step {
        out.push_str(" step ");
        format_expr(s, 0, out);
    }
}

/// Re-emit an f-string in escaped canonical form (spec §18.1): literal text is escaped and
/// `{`/`}` doubled so the output round-trips to the same value (raw-ness is not preserved).
fn format_fstring(parts: &[FStringPart], out: &mut String) {
    out.push('f');
    out.push('"');
    for p in parts {
        match p {
            FStringPart::Literal(s) => {
                for c in s.chars() {
                    match c {
                        '{' => out.push_str("{{"),
                        '}' => out.push_str("}}"),
                        _ => out.push_str(&escape_string(c)),
                    }
                }
            }
            FStringPart::Interp { expr, spec } => {
                out.push('{');
                // A formatted expression that starts with `{` (e.g. a dict literal or a postfix
                // over one) would lex as an escaped `{{`, and one ending with `}` would collide
                // with the closing `}` — insert spaces so the output re-parses to the same parts.
                let mut rendered = String::new();
                format_expr(expr, 0, &mut rendered);
                let leading = rendered.starts_with('{');
                let trailing = rendered.ends_with('}');
                if leading {
                    out.push(' ');
                }
                out.push_str(&rendered);
                if let Some(s) = spec {
                    out.push(':');
                    out.push_str(s);
                }
                if trailing {
                    out.push(' ');
                }
                out.push('}');
            }
        }
    }
    out.push('"');
}
