//! Import-source rendering for `prima fmt` (spec §15.3): the `import a::b as c;` and
//! `from a import b as d, e;` statements plus `::`-separated path emission. `format_path`
//! is also used by the expression printer for `a::b::c` path expressions.

use prima_syntax::ast::{Import, ImportItem, ImportKind, Spanned};

pub(crate) fn format_import(imp: &Import, out: &mut String) {
    match &imp.kind {
        ImportKind::Namespace { path, alias } => {
            out.push_str("import ");
            format_path(path, out);
            if let Some(a) = alias {
                out.push_str(" as ");
                out.push_str(&a.value);
            }
        }
        ImportKind::From { path, items } => {
            out.push_str("from ");
            format_path(path, out);
            out.push_str(" import ");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                match item {
                    ImportItem::Star => out.push('*'),
                    ImportItem::Name { name, alias } => {
                        out.push_str(&name.value);
                        if let Some(a) = alias {
                            out.push_str(" as ");
                            out.push_str(&a.value);
                        }
                    }
                }
            }
        }
    }
    out.push(';');
}

pub(crate) fn format_path(segments: &[Spanned<String>], out: &mut String) {
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            out.push_str("::");
        }
        out.push_str(&seg.value);
    }
}
