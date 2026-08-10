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
            _ => None,
        }
    }

    pub fn is_pure(self) -> bool {
        !matches!(self, Builtin::Print | Builtin::Println | Builtin::Simplify)
    }
}
