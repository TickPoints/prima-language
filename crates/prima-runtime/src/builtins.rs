/// Runtime builtin functions (spec §15.5 `core` pre-import): math operators, the collapse function family (spec §9),
/// string helpers (`to_string`/`concat`, §18.1; `format` was removed in v2.2), the `Option`/`Result` constructors (spec §4.4),
/// symbolic differentiation (`derivative`/`partial`/`grad`/`limit`, spec §19.4) and console I/O
/// (`print`/`println`/`input`/`read_line`, v2.1 §18.1b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Print,
    Println,
    Input,
    ReadLine,
    Simplify,
    Sqrt,
    Exp,
    Log,
    Ln,
    Sin,
    Cos,
    Tan,
    Abs,
    /// Symbolic differentiation (spec §19.4): dispatched through `eval_call` (accepts an MFn name
    /// or a symbolic expression); the variants never reach `call_builtin`.
    Derivative,
    Partial,
    Grad,
    Limit,
    /// `range(start, end, step = 1) -> Array` (spec §18.1b convenience): half-open integer range.
    Range,
    /// JIT compilation (spec §19.2): `jit(f)`/`jit(expr)`/`jit(grad(f))` produce a `Value::JitFunction`
    /// handle — dispatched through `eval_call` (the argument may be an MFn name or a symbolic expression),
    /// so it never reaches `call_builtin`.
    Jit,
    /// Collection convenience functions (spec appendix B.1, core pre-import): polymorphism over the
    /// collection types plus the explicit higher-order forms. `map`/`filter`/`reduce` never reach
    /// `call_builtin` — they are intercepted in `eval_call` so the function argument may be a name
    /// (functions are not first-class values).
    Len,
    Enumerate,
    Zip,
    Sorted,
    Reversed,
    Sum,
    Prod,
    Min,
    Max,
    All,
    Any,
    Join,
    Count,
    Index,
    First,
    Last,
    Linspace,
    Map,
    Filter,
    Reduce,
    Collapse(&'static str),
}

impl Builtin {
    pub fn from_name(name: &str) -> Option<Builtin> {
        match name {
            "print" => Some(Builtin::Print),
            "println" => Some(Builtin::Println),
            "input" => Some(Builtin::Input),
            "read_line" => Some(Builtin::ReadLine),
            "simplify" => Some(Builtin::Simplify),
            "sqrt" => Some(Builtin::Sqrt),
            "exp" => Some(Builtin::Exp),
            "log" => Some(Builtin::Log),
            "ln" => Some(Builtin::Ln),
            "sin" => Some(Builtin::Sin),
            "cos" => Some(Builtin::Cos),
            "tan" => Some(Builtin::Tan),
            "abs" => Some(Builtin::Abs),
            // Symbolic differentiation (spec §19.4): handled by `eval_call`, which accepts an MFn name
            // or a symbolic expression and evaluates the variable argument as a symbol.
            "derivative" => Some(Builtin::Derivative),
            "partial" => Some(Builtin::Partial),
            "grad" => Some(Builtin::Grad),
            "limit" => Some(Builtin::Limit),
            "range" => Some(Builtin::Range),
            // JIT compilation (spec §19.2): intercepted in `eval_call` like `derivative` — the
            // argument is an MFn name or a symbolic expression, not a plain value.
            "jit" => Some(Builtin::Jit),
            // Collection convenience functions (spec appendix B.1, core pre-import).
            "len" => Some(Builtin::Len),
            "enumerate" => Some(Builtin::Enumerate),
            "zip" => Some(Builtin::Zip),
            "sorted" => Some(Builtin::Sorted),
            "reversed" => Some(Builtin::Reversed),
            "sum" => Some(Builtin::Sum),
            "prod" => Some(Builtin::Prod),
            "min" => Some(Builtin::Min),
            "max" => Some(Builtin::Max),
            "all" => Some(Builtin::All),
            "any" => Some(Builtin::Any),
            "join" => Some(Builtin::Join),
            "count" => Some(Builtin::Count),
            "index" => Some(Builtin::Index),
            "first" => Some(Builtin::First),
            "last" => Some(Builtin::Last),
            "linspace" => Some(Builtin::Linspace),
            "map" => Some(Builtin::Map),
            "filter" => Some(Builtin::Filter),
            "reduce" => Some(Builtin::Reduce),
            // Collapse function family (spec §9): `to_/try_/checked_/clamped_/rounded_/truncated_` plus the `unwrap` family.
            "to_i8" => Some(Builtin::Collapse("to_i8")),
            "to_i16" => Some(Builtin::Collapse("to_i16")),
            "to_i32" => Some(Builtin::Collapse("to_i32")),
            "to_i64" => Some(Builtin::Collapse("to_i64")),
            "to_i128" => Some(Builtin::Collapse("to_i128")),
            "to_u8" => Some(Builtin::Collapse("to_u8")),
            "to_u16" => Some(Builtin::Collapse("to_u16")),
            "to_u32" => Some(Builtin::Collapse("to_u32")),
            "to_u64" => Some(Builtin::Collapse("to_u64")),
            "to_u128" => Some(Builtin::Collapse("to_u128")),
            "to_isize" => Some(Builtin::Collapse("to_isize")),
            "to_usize" => Some(Builtin::Collapse("to_usize")),
            "to_f32" => Some(Builtin::Collapse("to_f32")),
            "to_f64" => Some(Builtin::Collapse("to_f64")),
            "to_bigint" => Some(Builtin::Collapse("to_bigint")),
            "to_rational" => Some(Builtin::Collapse("to_rational")),
            "to_bigfloat" => Some(Builtin::Collapse("to_bigfloat")),
            "to_complex" => Some(Builtin::Collapse("to_complex")),
            "try_i8" => Some(Builtin::Collapse("try_i8")),
            "try_i16" => Some(Builtin::Collapse("try_i16")),
            "try_i32" => Some(Builtin::Collapse("try_i32")),
            "try_i64" => Some(Builtin::Collapse("try_i64")),
            "try_i128" => Some(Builtin::Collapse("try_i128")),
            "try_u8" => Some(Builtin::Collapse("try_u8")),
            "try_u16" => Some(Builtin::Collapse("try_u16")),
            "try_u32" => Some(Builtin::Collapse("try_u32")),
            "try_u64" => Some(Builtin::Collapse("try_u64")),
            "try_u128" => Some(Builtin::Collapse("try_u128")),
            "try_isize" => Some(Builtin::Collapse("try_isize")),
            "try_usize" => Some(Builtin::Collapse("try_usize")),
            "try_f32" => Some(Builtin::Collapse("try_f32")),
            "try_f64" => Some(Builtin::Collapse("try_f64")),
            "try_bigint" => Some(Builtin::Collapse("try_bigint")),
            "try_rational" => Some(Builtin::Collapse("try_rational")),
            "try_complex" => Some(Builtin::Collapse("try_complex")),
            "checked_i8" => Some(Builtin::Collapse("checked_i8")),
            "checked_i16" => Some(Builtin::Collapse("checked_i16")),
            "checked_i32" => Some(Builtin::Collapse("checked_i32")),
            "checked_i64" => Some(Builtin::Collapse("checked_i64")),
            "checked_i128" => Some(Builtin::Collapse("checked_i128")),
            "checked_u8" => Some(Builtin::Collapse("checked_u8")),
            "checked_u16" => Some(Builtin::Collapse("checked_u16")),
            "checked_u32" => Some(Builtin::Collapse("checked_u32")),
            "checked_u64" => Some(Builtin::Collapse("checked_u64")),
            "checked_u128" => Some(Builtin::Collapse("checked_u128")),
            "checked_add" => Some(Builtin::Collapse("checked_add")),
            "checked_mul" => Some(Builtin::Collapse("checked_mul")),
            "clamped_i8" => Some(Builtin::Collapse("clamped_i8")),
            "clamped_i16" => Some(Builtin::Collapse("clamped_i16")),
            "clamped_i32" => Some(Builtin::Collapse("clamped_i32")),
            "clamped_i64" => Some(Builtin::Collapse("clamped_i64")),
            "clamped_i128" => Some(Builtin::Collapse("clamped_i128")),
            "clamped_u8" => Some(Builtin::Collapse("clamped_u8")),
            "clamped_u16" => Some(Builtin::Collapse("clamped_u16")),
            "clamped_u32" => Some(Builtin::Collapse("clamped_u32")),
            "clamped_u64" => Some(Builtin::Collapse("clamped_u64")),
            "clamped_u128" => Some(Builtin::Collapse("clamped_u128")),
            "clamped_f32" => Some(Builtin::Collapse("clamped_f32")),
            "clamped_f64" => Some(Builtin::Collapse("clamped_f64")),
            "rounded_f64" => Some(Builtin::Collapse("rounded_f64")),
            "rounded_f32" => Some(Builtin::Collapse("rounded_f32")),
            "rounded_i32" => Some(Builtin::Collapse("rounded_i32")),
            "truncated_i32" => Some(Builtin::Collapse("truncated_i32")),
            "unwrap" => Some(Builtin::Collapse("unwrap")),
            "unwrap_or" => Some(Builtin::Collapse("unwrap_or")),
            "expect" => Some(Builtin::Collapse("expect")),
            // String/format helpers (spec §18.1). `format` was removed in v2.2 (f-strings replace it).
            "to_string" => Some(Builtin::Collapse("to_string")),
            "concat" => Some(Builtin::Collapse("concat")),
            // Option/Result constructors (spec §4.4).
            "Some" => Some(Builtin::Collapse("Some")),
            "None" => Some(Builtin::Collapse("None")),
            "Ok" => Some(Builtin::Collapse("Ok")),
            "Err" => Some(Builtin::Collapse("Err")),
            _ => None,
        }
    }

    pub fn is_pure(self) -> bool {
        !matches!(
            self,
            Builtin::Print | Builtin::Println | Builtin::Input | Builtin::ReadLine | Builtin::Simplify
        )
    }

    /// Collection convenience functions take their array argument **whole** — the implicit-broadcast
    /// path (spec §11.4) must not split the array for `len`/`sum`/`sorted`/… (spec appendix B.1).
    pub fn is_collection(self) -> bool {
        matches!(
            self,
            Builtin::Len
                | Builtin::Enumerate
                | Builtin::Zip
                | Builtin::Sorted
                | Builtin::Reversed
                | Builtin::Sum
                | Builtin::Prod
                | Builtin::Min
                | Builtin::Max
                | Builtin::All
                | Builtin::Any
                | Builtin::Join
                | Builtin::Count
                | Builtin::Index
                | Builtin::First
                | Builtin::Last
                | Builtin::Linspace
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Builtin;

    #[test]
    fn collapse_names_register() {
        for name in [
            "to_i8",
            "to_i16",
            "to_i32",
            "to_i64",
            "to_i128",
            "to_u8",
            "to_u16",
            "to_u32",
            "to_u64",
            "to_u128",
            "to_isize",
            "to_usize",
            "to_f32",
            "to_f64",
            "to_bigint",
            "to_rational",
            "to_bigfloat",
            "to_complex",
            "try_i8",
            "try_i16",
            "try_i32",
            "try_i64",
            "try_i128",
            "try_u8",
            "try_u16",
            "try_u32",
            "try_u64",
            "try_u128",
            "try_isize",
            "try_usize",
            "try_f32",
            "try_f64",
            "try_bigint",
            "try_rational",
            "try_complex",
            "checked_i8",
            "checked_i16",
            "checked_i32",
            "checked_i64",
            "checked_i128",
            "checked_u8",
            "checked_u16",
            "checked_u32",
            "checked_u64",
            "checked_u128",
            "checked_add",
            "checked_mul",
            "clamped_i8",
            "clamped_i16",
            "clamped_i32",
            "clamped_i64",
            "clamped_i128",
            "clamped_u8",
            "clamped_u16",
            "clamped_u32",
            "clamped_u64",
            "clamped_u128",
            "clamped_f32",
            "clamped_f64",
            "rounded_f64",
            "rounded_f32",
            "rounded_i32",
            "truncated_i32",
            "unwrap",
            "unwrap_or",
            "expect",
            "to_string",
            "concat",
            "Some",
            "None",
            "Ok",
            "Err",
        ] {
            assert_eq!(Builtin::from_name(name), Some(Builtin::Collapse(name)), "name = {name}");
        }
    }
}
