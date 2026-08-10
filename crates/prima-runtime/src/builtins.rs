/// Runtime builtin functions (spec §15.5 `core` pre-import): math operators and the collapse function family (spec §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Print,
    Println,
    Simplify,
    Sqrt,
    Exp,
    Log,
    Ln,
    Sin,
    Cos,
    Tan,
    Abs,
    Collapse(&'static str),
}

impl Builtin {
    pub fn from_name(name: &str) -> Option<Builtin> {
        match name {
            "print" => Some(Builtin::Print),
            "println" => Some(Builtin::Println),
            "simplify" => Some(Builtin::Simplify),
            "sqrt" => Some(Builtin::Sqrt),
            "exp" => Some(Builtin::Exp),
            "log" => Some(Builtin::Log),
            "ln" => Some(Builtin::Ln),
            "sin" => Some(Builtin::Sin),
            "cos" => Some(Builtin::Cos),
            "tan" => Some(Builtin::Tan),
            "abs" => Some(Builtin::Abs),
            // Collapse function family (spec §9): `to_/try_/checked_/clamped_/rounded_/truncated_` plus the `unwrap` family.
            "to_i32" => Some(Builtin::Collapse("to_i32")),
            "to_i64" => Some(Builtin::Collapse("to_i64")),
            "to_f32" => Some(Builtin::Collapse("to_f32")),
            "to_f64" => Some(Builtin::Collapse("to_f64")),
            "to_bigint" => Some(Builtin::Collapse("to_bigint")),
            "to_rational" => Some(Builtin::Collapse("to_rational")),
            "to_bigfloat" => Some(Builtin::Collapse("to_bigfloat")),
            "to_complex" => Some(Builtin::Collapse("to_complex")),
            "try_i32" => Some(Builtin::Collapse("try_i32")),
            "try_i64" => Some(Builtin::Collapse("try_i64")),
            "try_f64" => Some(Builtin::Collapse("try_f64")),
            "try_bigint" => Some(Builtin::Collapse("try_bigint")),
            "try_rational" => Some(Builtin::Collapse("try_rational")),
            "try_complex" => Some(Builtin::Collapse("try_complex")),
            "checked_i32" => Some(Builtin::Collapse("checked_i32")),
            "checked_u64" => Some(Builtin::Collapse("checked_u64")),
            "checked_add" => Some(Builtin::Collapse("checked_add")),
            "checked_mul" => Some(Builtin::Collapse("checked_mul")),
            "clamped_i32" => Some(Builtin::Collapse("clamped_i32")),
            "clamped_u64" => Some(Builtin::Collapse("clamped_u64")),
            "clamped_f64" => Some(Builtin::Collapse("clamped_f64")),
            "rounded_f64" => Some(Builtin::Collapse("rounded_f64")),
            "rounded_i32" => Some(Builtin::Collapse("rounded_i32")),
            "truncated_i32" => Some(Builtin::Collapse("truncated_i32")),
            "unwrap" => Some(Builtin::Collapse("unwrap")),
            "unwrap_or" => Some(Builtin::Collapse("unwrap_or")),
            "expect" => Some(Builtin::Collapse("expect")),
            _ => None,
        }
    }

    pub fn is_pure(self) -> bool {
        !matches!(self, Builtin::Print | Builtin::Println | Builtin::Simplify)
    }
}

#[cfg(test)]
mod tests {
    use super::Builtin;

    #[test]
    fn collapse_names_register() {
        for name in [
            "to_i32",
            "to_f64",
            "try_complex",
            "checked_mul",
            "clamped_f64",
            "rounded_f64",
            "truncated_i32",
            "unwrap",
            "unwrap_or",
            "expect",
        ] {
            assert_eq!(Builtin::from_name(name), Some(Builtin::Collapse(name)), "name = {name}");
        }
    }
}
