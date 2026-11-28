//! Arc method support for user classes and builtin values (spec §4.5/§11/§18).

use super::*;

impl Evaluator {
    /// Raise the explicit count of an `Arc` instance by one and return the same handle
    /// (`mem::Arc::clone`).
    pub fn mem_arc_clone(&mut self, arc: Value) -> Result<Value, RuntimeError> {
        let id = match &arc {
            Value::Class(id) => *id,
            other => {
                return Err(RuntimeError::Type(format!(
                    "`Arc.clone` expects an Arc receiver, got {}",
                    value_type_name(other)
                )));
            }
        };
        let instance = self
            .instances
            .get_mut(&id)
            .ok_or_else(|| RuntimeError::UnknownClassInstance)?;
        if instance.class != "Arc" {
            return Err(RuntimeError::Type(format!(
                "`Arc.clone` expects an Arc receiver, got `{}`",
                instance.class
            )));
        }
        let count = match instance.fields.get("count") {
            Some(Value::Number(n)) => n.as_i64().ok_or_else(|| RuntimeError::Internal {
                problem: crate::error::InternalProblem::ArcCountNotInteger,
            })?,
            _ => {
                return Err(RuntimeError::Internal {
                    problem: crate::error::InternalProblem::ArcMissingField {
                        field: "count".into(),
                    },
                });
            }
        };
        let next = count
            .checked_add(1)
            .ok_or_else(|| RuntimeError::Overflow("`Arc` reference count overflowed".into()))?;
        instance
            .fields
            .insert("count".into(), Value::Number(Number::from(next)));
        Ok(arc)
    }

    /// The payload held by an `Arc` (`mem::Arc::get`).
    pub fn mem_arc_payload(&self, arc: &Value) -> Result<Value, RuntimeError> {
        self.mem_arc_field(arc, "value", "Arc.get")
    }

    /// The explicit count of an `Arc` (`mem::Arc::strong_count`).
    pub fn mem_arc_count(&self, arc: &Value) -> Result<Value, RuntimeError> {
        self.mem_arc_field(arc, "count", "Arc.strong_count")
    }

    /// Shared field lookup for the `mem::Arc` accessors.
    pub(crate) fn mem_arc_field(
        &self,
        arc: &Value,
        field: &str,
        method: &str,
    ) -> Result<Value, RuntimeError> {
        let Value::Class(id) = arc else {
            return Err(RuntimeError::Type(format!(
                "`{method}` expects an Arc receiver, got {}",
                value_type_name(arc)
            )));
        };
        let instance = self
            .instances
            .get(id)
            .ok_or_else(|| RuntimeError::UnknownClassInstance)?;
        if instance.class != "Arc" {
            return Err(RuntimeError::Type(format!(
                "`{method}` expects an Arc receiver, got `{}`",
                instance.class
            )));
        }
        instance
            .fields
            .get(field)
            .cloned()
            .ok_or_else(|| RuntimeError::Internal {
                problem: crate::error::InternalProblem::ArcMissingField {
                    field: field.to_string(),
                },
            })
    }
}
