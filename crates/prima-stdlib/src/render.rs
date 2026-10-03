//! `render` module (spec §18.6): render symbolic expressions as formula images and terminal text.
//!
//! `to_svg` reuses the core LaTeX view ([`prima_core::render_latex`]) and feeds it through the
//! RaTeX pipeline (`ratex-parser` → `ratex-layout` → `ratex-svg`) to produce a self-contained SVG
//! document (glyphs embedded as outlines, so no KaTeX stylesheet is needed).
//! `to_terminal` transliterates the same LaTeX view into Unicode terminal math text.
//!
//! Both accept a symbolic `Value::Expr`/`Value::Symbol` or a `String` containing LaTeX.

use prima_core::{Value, render_latex};
use prima_runtime::builtin;
use prima_runtime::{Evaluator, RuntimeError};

fn arity(args: &[Value], n: usize, fname: &str) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(RuntimeError::Message(format!(
            "`{fname}` expects {n} argument(s), got {}",
            args.len()
        )))
    }
}

/// The LaTeX source for the single `render` argument: an `Expr`/`Symbol` through the core renderer,
/// or a `String` taken as LaTeX source directly.
fn arg_latex(ev: &Evaluator, v: &Value, fname: &str) -> Result<String, RuntimeError> {
    match v {
        Value::Expr(id) => Ok(render_latex(ev.pool(), ev.symbols(), *id)),
        Value::Symbol(s) => Ok(ev
            .symbols()
            .name(prima_core::SymbolId(*s))
            .unwrap_or_else(|| format!("?{s}"))),
        Value::String(s) => Ok(s.to_string()),
        other => Err(RuntimeError::Type(format!(
            "`{fname}` expects a symbolic expression or a LaTeX string, got {other:?}"
        ))),
    }
}

/// Register the `render` `@builtin` implementations (spec §18.4/§18.6).
pub fn register() {
    builtin!("render::to_svg", to_svg);
    builtin!("render::to_terminal", to_terminal);
}

/// `render::to_svg(expr)` → `Result<String, String>` (self-contained SVG).
fn to_svg(ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "render::to_svg")?;
    let latex = arg_latex(ev, &args[0], "render::to_svg")?;
    match latex_to_svg(&latex) {
        Ok(svg) => Ok(Value::Result(Ok(Box::new(Value::String(svg.into()))))),
        Err(e) => Ok(Value::Result(Err(Box::new(e)))),
    }
}

/// `render::to_terminal(expr)` → Unicode math text.
fn to_terminal(ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "render::to_terminal")?;
    let latex = arg_latex(ev, &args[0], "render::to_terminal")?;
    Ok(Value::String(latex_to_unicode(&latex).into()))
}

/// LaTeX → self-contained SVG via the RaTeX pipeline (spec §18.6).
fn latex_to_svg(latex: &str) -> Result<String, String> {
    let nodes = ratex_parser::parse(latex).map_err(|e| e.to_string())?;
    let options = ratex_layout::LayoutOptions::default();
    let layout = ratex_layout::layout(&nodes, &options);
    let display = ratex_layout::to_display_list(&layout);
    let svg_options = ratex_svg::SvgOptions {
        embed_glyphs: true,
        ..Default::default()
    };
    Ok(ratex_svg::render_to_svg(&display, &svg_options))
}

// —— LaTeX → Unicode terminal text ——

/// Transliterate the LaTeX subset emitted by [`prima_core::render_latex`] into Unicode math text.
/// Unsupported constructs degrade to readable ASCII rather than failing.
fn latex_to_unicode(latex: &str) -> String {
    let mut out = String::new();
    transliterate(&mut latex.chars().peekable(), &mut out);
    out
}

// The loop body hands the same iterator to `command`/`script`, so `while let` is required here
// (a `for c in chars.by_ref()` loop would hold the mutable borrow across them).
#[allow(clippy::while_let_on_iterator)]
fn transliterate(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, out: &mut String) {
    while let Some(c) = chars.next() {
        match c {
            '\\' => command(chars, out),
            '^' => script(chars, out, true),
            '_' => script(chars, out, false),
            '{' | '}' => {} // grouping braces are invisible in the terminal view
            _ => out.push(c),
        }
    }
}

/// Read a `{...}` group (the leading `{` already consumed) and return its raw contents.
fn read_group(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut depth = 1usize;
    let mut group = String::new();
    for c in chars.by_ref() {
        match c {
            '{' => {
                depth += 1;
                group.push(c);
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                group.push(c);
            }
            _ => group.push(c),
        }
    }
    group
}

/// Read the next argument: either a `{...}` group or a single token.
fn read_arg(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    match chars.peek() {
        Some('{') => {
            chars.next();
            read_group(chars)
        }
        Some(_) => chars.next().map(String::from).unwrap_or_default(),
        None => String::new(),
    }
}

/// Handle a `\command`, translating common math names and the structural commands `frac`/`sqrt`/
/// `left`/`right`.
fn command(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, out: &mut String) {
    let mut name = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphabetic() {
            name.push(c);
            chars.next();
        } else {
            break;
        }
    }
    match name.as_str() {
        "frac" => {
            let num = read_arg(chars);
            let den = read_arg(chars);
            out.push('(');
            transliterate(&mut num.chars().peekable(), out);
            out.push_str(")/(");
            transliterate(&mut den.chars().peekable(), out);
            out.push(')');
        }
        "sqrt" => {
            let arg = read_arg(chars);
            out.push('√');
            let needs_paren = !arg.chars().all(char::is_alphanumeric);
            if needs_paren {
                out.push('(');
            }
            transliterate(&mut arg.chars().peekable(), out);
            if needs_paren {
                out.push(')');
            }
        }
        "left" | "right" => {
            // Drop the command and keep the delimiter that follows.
            if let Some(d) = chars.next() {
                out.push(d);
            }
        }
        _ => {
            if let Some(sym) = greek_or_symbol(&name) {
                out.push_str(sym);
            } else {
                // A known function name or an unknown command: keep the bare name.
                out.push_str(&name);
            }
        }
    }
}

/// Unicode superscripts/subscripts for `^{...}` / `_{...}`. Non-simple groups fall back to `^(...)`.
fn script(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, out: &mut String, sup: bool) {
    let arg = read_arg(chars);
    if let Some(mapped) = map_script(&arg, sup) {
        out.push_str(&mapped);
    } else {
        out.push(if sup { '^' } else { '_' });
        out.push('(');
        transliterate(&mut arg.chars().peekable(), out);
        out.push(')');
    }
}

fn map_script(s: &str, sup: bool) -> Option<String> {
    let table: &[(char, char)] = if sup {
        &[
            ('0', '⁰'),
            ('1', '¹'),
            ('2', '²'),
            ('3', '³'),
            ('4', '⁴'),
            ('5', '⁵'),
            ('6', '⁶'),
            ('7', '⁷'),
            ('8', '⁸'),
            ('9', '⁹'),
            ('+', '⁺'),
            ('-', '⁻'),
            ('=', '⁼'),
            ('(', '⁽'),
            (')', '⁾'),
            ('n', 'ⁿ'),
            ('i', 'ⁱ'),
        ]
    } else {
        &[
            ('0', '₀'),
            ('1', '₁'),
            ('2', '₂'),
            ('3', '₃'),
            ('4', '₄'),
            ('5', '₅'),
            ('6', '₆'),
            ('7', '₇'),
            ('8', '₈'),
            ('9', '₉'),
            ('+', '₊'),
            ('-', '₋'),
            ('=', '₌'),
            ('(', '₍'),
            (')', '₎'),
        ]
    };
    let mut mapped = String::new();
    for c in s.chars() {
        mapped.push(
            table
                .iter()
                .find(|(from, _)| *from == c)
                .map(|(_, to)| *to)?,
        );
    }
    if mapped.is_empty() {
        None
    } else {
        Some(mapped)
    }
}

/// Greek letters and common operator symbols (spec §7 TeX names).
fn greek_or_symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" => "ρ",
        "sigma" => "σ",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "cdot" => "·",
        "times" => "×",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "le" | "leq" => "≤",
        "ge" | "geq" => "≥",
        "ne" | "neq" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "to" | "rightarrow" => "→",
        "leftarrow" => "←",
        "infty" => "∞",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "partial" => "∂",
        "nabla" => "∇",
        "forall" => "∀",
        "exists" => "∃",
        "in" => "∈",
        "notin" => "∉",
        "subset" => "⊂",
        "supset" => "⊃",
        "cup" => "∪",
        "cap" => "∩",
        "emptyset" => "∅",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_frac_and_pow() {
        assert_eq!(latex_to_unicode(r"\frac{1}{2}"), "(1)/(2)");
        assert_eq!(latex_to_unicode("x^{2}"), "x²");
        assert_eq!(latex_to_unicode("x^{n+1}"), "xⁿ⁺¹");
        assert_eq!(latex_to_unicode("x^{2 y}"), "x^(2 y)");
    }

    #[test]
    fn terminal_sqrt_and_greek() {
        assert_eq!(latex_to_unicode(r"\sqrt{2}"), "√2");
        assert_eq!(latex_to_unicode(r"\sqrt{x + 1}"), "√(x + 1)");
        assert_eq!(latex_to_unicode(r"\pi"), "π");
    }

    #[test]
    fn terminal_delimiters_strip() {
        assert_eq!(latex_to_unicode(r"\left(x + 1\right)"), "(x + 1)");
    }

    #[test]
    fn svg_is_self_contained() {
        let svg = latex_to_svg(r"\frac{a}{b}").expect("svg");
        assert!(svg.contains("<svg"), "not svg: {svg}");
    }
}
