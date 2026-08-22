//! Class model (spec §4.5/§12.3): the class registry held by the evaluator.
//!
//! `ClassDef` records the field list and method table of a class; `ClassInstance` is the runtime
//! object referenced by `Value::Class(id)` (spec §5). `self` is a shallow copy (shared handle), so
//! methods receive the same instance id as the receiver (spec §12.3).

use std::collections::HashMap;

use prima_core::Value;
use prima_syntax::ast::{Block, DocComment, Param, Type, Visibility};

use crate::eval::EnvRef;

/// A field definition: name → declared type + visibility (spec §4.5).
#[derive(Clone)]
pub struct FieldDef {
    pub ty: Type,
    pub vis: Visibility,
}

/// A method definition (spec §4.5): parameters, optional return type, body (None for an
/// `@builtin` signature, spec §18.4), visibility, and the `///` doc comment (spec §4.1) used for
/// the failed-call diagnostic note (spec §16.4). The captured environment is the one in which
/// the class was defined, so method bodies resolve module-local functions.
#[derive(Clone)]
pub struct MethodDef {
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: Option<Block>,
    pub vis: Visibility,
    pub env: EnvRef,
    /// `///` doc comment preceding the method (spec §4.1).
    pub docs: Option<DocComment>,
}

/// Class definition (spec §4.7 class registry): field/method tables indexed by name.
#[derive(Clone)]
pub struct ClassDef {
    pub name: String,
    /// Module path the class was defined in (`""` for the entry/root module), for `pub(mod)` visibility (spec §15.2).
    pub module: String,
    pub fields: HashMap<String, FieldDef>,
    pub methods: HashMap<String, MethodDef>,
    /// `///` doc comment preceding the class (spec §4.1).
    pub docs: Option<DocComment>,
}

/// A runtime class instance: the owning class name plus the field map (spec §12.3).
#[derive(Debug, Clone)]
pub struct ClassInstance {
    pub class: String,
    pub fields: HashMap<String, Value>,
}
