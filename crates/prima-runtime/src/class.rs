//! Class model (spec §4.5/§12.3): the class registry held by the evaluator.
//!
//! `ClassDef` records the field list and method table of a class; `ClassInstance` is the runtime
//! object referenced by `Value::Class(id)` (spec §5). `self` is a shallow copy (shared handle), so
//! methods receive the same instance id as the receiver (spec §12.3).

use std::collections::HashMap;

use prima_core::Value;
use prima_syntax::ast::{Block, DocComment, Param, Type, Visibility};

use crate::eval::{EnvRef, NativeCall};

/// A field definition: name → declared type + visibility (spec §4.5).
#[derive(Clone)]
pub struct FieldDef {
    pub ty: Type,
    pub vis: Visibility,
}

/// How a builtin-class method is dispatched (spec §18.1). `Plain` methods are resolved through a
/// registered native implementation and/or a `.pra` body; the other natures dispatch to a
/// runtime backend that predates the method-system work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodNature {
    /// Registered native impl and/or `.pra` body (String; user classes; layered methods).
    Plain,
    /// Read-only native method dispatched through the value-type backend
    /// (`call_array_method`/`call_dict_method`/`call_set_method`/char/tuple, spec §11.3/§11.6).
    Backend,
    /// Mutating method that writes back through the receiver binding (`mutate_*`, spec §11.3/§11.6).
    Mutating,
    /// Numeric method dispatched through the collapse family (`collapse::call`, spec §9).
    Collapse,
}

/// A method definition (spec §4.5): parameters, optional return type, body (None for an
/// `@builtin` signature, spec §18.4), visibility, and the `///` doc comment (spec §4.1) used for
/// the failed-call diagnostic note (spec §16.4). The captured environment is the one in which
/// the class was defined, so method bodies resolve module-local functions.
///
/// v2.2 (Phase 10) added the layered-optimization fields: `native` is the registered Rust
/// implementation and `level` the `@builtin(ON)` tier (spec §18.4). A layered method uses `native`
/// when `opt_level >= level` and evaluates its `.pra` body otherwise; `nature` selects the dispatch
/// backend for the builtin classes (spec §18.1).
#[derive(Clone)]
pub struct MethodDef {
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: Option<Block>,
    /// Registered Rust implementation, keyed `"<Class>::<method>"` (spec §18.4); `None` for pure
    /// `.pra` methods and for methods dispatched through a runtime backend.
    pub native: Option<NativeCall>,
    /// `@builtin(ON)` layered-optimization tier (`0` = plain method or `@builtin` O0, spec §18.4).
    pub level: u8,
    /// Dispatch backend for builtin-class methods (spec §18.1).
    pub nature: MethodNature,
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
