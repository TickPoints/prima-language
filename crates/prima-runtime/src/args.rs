//! Shared positional-argument validation for host callables (spec §18.4): builtin functions,
//! `@builtin` implementations, and the collapse family. One implementation keeps arity and type
//! diagnostics identical everywhere.

use std::collections::{HashMap, HashSet};

use prima_core::{Number, Value, ValueKey};

use crate::error::{Arity, RuntimeError};

/// A borrowed argument list plus the callable name used in diagnostics.
pub struct Args<'a> {
    callable: &'a str,
    values: &'a [Value],
}

impl<'a> Args<'a> {
    pub fn new(callable: &'a str, values: &'a [Value]) -> Self {
        Self { callable, values }
    }

    /// Require exactly `expected` positional arguments.
    pub fn exact(&self, expected: usize) -> Result<(), RuntimeError> {
        if self.values.len() == expected {
            Ok(())
        } else {
            Err(RuntimeError::Arity {
                callable: self.callable.into(),
                expected: Arity::Exact(expected),
                got: self.values.len(),
            })
        }
    }

    /// Require at least `min` positional arguments.
    pub fn at_least(&self, min: usize) -> Result<(), RuntimeError> {
        if self.values.len() >= min {
            Ok(())
        } else {
            Err(RuntimeError::Arity {
                callable: self.callable.into(),
                expected: Arity::AtLeast(min),
                got: self.values.len(),
            })
        }
    }

    /// Require one of a small closed set of argument counts (e.g. `1` or `2`).
    pub fn one_of(&self, options: &'static [usize]) -> Result<(), RuntimeError> {
        if options.contains(&self.values.len()) {
            Ok(())
        } else {
            Err(RuntimeError::Arity {
                callable: self.callable.into(),
                expected: Arity::OneOf(options),
                got: self.values.len(),
            })
        }
    }

    /// Require exactly `expected` arguments after an implicit receiver (method calls).
    pub fn exact_after_receiver(&self, expected: usize) -> Result<(), RuntimeError> {
        let got = self.values.len().saturating_sub(1);
        if got == expected {
            Ok(())
        } else {
            Err(RuntimeError::Arity {
                callable: self.callable.into(),
                expected: Arity::Exact(expected),
                got,
            })
        }
    }

    /// The argument at `position`, or a missing-argument diagnostic.
    pub fn get(&self, position: usize) -> Result<&Value, RuntimeError> {
        self.values
            .get(position)
            .ok_or_else(|| RuntimeError::MissingArgument {
                callable: self.callable.into(),
                position,
            })
    }

    pub fn number(&self, position: usize) -> Result<&Number, RuntimeError> {
        match self.get(position)? {
            Value::Number(n) => Ok(n),
            other => Err(self.argument_type(position, "a number", type_name(other))),
        }
    }

    /// The argument as an `f64` via lossy collapse (spec §B.3 numeric interfaces).
    pub fn real(&self, position: usize) -> Result<f64, RuntimeError> {
        Ok(self.number(position)?.to_f64_lossy())
    }

    pub fn i64(&self, position: usize) -> Result<i64, RuntimeError> {
        match self.get(position)? {
            Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| self.argument_type(position, "an integer", n.to_string())),
            other => Err(self.argument_type(position, "an integer", type_name(other))),
        }
    }

    pub fn bigint(&self, position: usize) -> Result<num_bigint::BigInt, RuntimeError> {
        match self.get(position)? {
            Value::Number(n) => n
                .as_bigint()
                .ok_or_else(|| self.argument_type(position, "an integer", n.to_string())),
            other => Err(self.argument_type(position, "an integer", type_name(other))),
        }
    }

    pub fn string(&self, position: usize) -> Result<String, RuntimeError> {
        match self.get(position)? {
            Value::String(s) => Ok(s.to_string()),
            other => Err(self.argument_type(position, "a string", type_name(other))),
        }
    }

    pub fn character(&self, position: usize) -> Result<char, RuntimeError> {
        match self.get(position)? {
            Value::Char(c) => Ok(*c),
            other => Err(self.argument_type(position, "a char", type_name(other))),
        }
    }

    pub fn array(&self, position: usize) -> Result<Vec<Value>, RuntimeError> {
        match self.get(position)? {
            Value::Array(a) => Ok(a.to_vec()),
            other => Err(self.argument_type(position, "an array", type_name(other))),
        }
    }

    /// A numeric array; a non-numeric element reports that element's type.
    pub fn f64_array(&self, position: usize) -> Result<Vec<f64>, RuntimeError> {
        let items = self.array(position)?;
        items
            .iter()
            .map(|item| match item {
                Value::Number(n) => Ok(n.to_f64_lossy()),
                other => Err(self.argument_type(position, "an array of numbers", type_name(other))),
            })
            .collect()
    }

    pub fn string_array(&self, position: usize) -> Result<Vec<String>, RuntimeError> {
        let items = self.array(position)?;
        items
            .iter()
            .map(|item| match item {
                Value::String(s) => Ok(s.to_string()),
                other => Err(self.argument_type(position, "an array of strings", type_name(other))),
            })
            .collect()
    }

    pub fn set(&self, position: usize) -> Result<&HashSet<ValueKey>, RuntimeError> {
        match self.get(position)? {
            Value::Set(set) => Ok(set),
            other => Err(self.argument_type(position, "a set", type_name(other))),
        }
    }

    pub fn dict(&self, position: usize) -> Result<&HashMap<ValueKey, Value>, RuntimeError> {
        match self.get(position)? {
            Value::Dict(dict) => Ok(dict),
            other => Err(self.argument_type(position, "a dict", type_name(other))),
        }
    }

    fn argument_type(
        &self,
        position: usize,
        expected: &'static str,
        got: impl Into<String>,
    ) -> RuntimeError {
        RuntimeError::argument_type(self.callable.into(), position, expected, got.into())
    }
}

fn type_name(value: &Value) -> String {
    crate::eval::value_type_name(value)
}
