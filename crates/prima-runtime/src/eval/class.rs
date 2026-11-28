//! Class and value-type dispatch (spec §4.5/§5/§12/§18): class resolution, associated functions,
//! method/field access, struct literals, and builtin dispatch for `String`/`Array`/`Dict`/`Set`/
//! `Char`/`Tuple` plus their mutating operations.

use super::helpers::{
    did_you_mean, is_mutating_array_method, is_mutating_dict_method, is_mutating_set_method,
    method_note, native_method_error, normalize_index, normalize_insert, numeric_method_name,
    path_key, unalias_array_cycles, value_contains_array_buffer, with_notes,
};
use super::*;
use prima_core::suggest::did_you_mean_help;

impl Evaluator {
    /// Resolve a class name: `T` (local registry) or `mod::T` (module export, spec §15.2).
    pub(crate) fn resolve_class(
        &self,
        env: &EnvRef,
        segments: &[Spanned<String>],
    ) -> Option<ClassDef> {
        if segments.is_empty() {
            return None;
        }
        let name = &segments[segments.len() - 1].value;
        if segments.len() == 1 {
            self.class_defs.get(name).cloned()
        } else {
            let ns = path_key(&segments[..segments.len() - 1]);
            match env.borrow().lookup_module_item(&ns, name) {
                Some(NamespaceItem::Class(def)) => Some(def),
                _ => self.class_defs.get(name).cloned(),
            }
        }
    }

    /// Call an associated function `T::name(args)` (spec §4.5): a method with no `self` parameter.
    pub(crate) fn call_associated(
        &mut self,
        def: &ClassDef,
        method_name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        crate::error::catch_propagate(self.call_associated_inner(def, method_name, args))
    }

    fn call_associated_inner(
        &mut self,
        def: &ClassDef,
        method_name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let method = match def.methods.get(method_name) {
            Some(m) => m.clone(),
            None => {
                let err = RuntimeError::UnknownAssociatedFunction {
                    class: def.name.clone(),
                    name: method_name.to_string(),
                };
                // Attach a `did you mean` help and a doc note from the nearest candidate (spec §16.4).
                let mut notes = Vec::new();
                let mut help = None;
                if let Some(cand) =
                    did_you_mean(method_name, def.methods.keys().map(|k| k.as_str()))
                {
                    if cand != method_name {
                        help = Some(format!("did you mean `{cand}`?"));
                    }
                    if let Some(m) = def.methods.get(&cand) {
                        notes.extend(method_note(&cand, m));
                    }
                }
                return Err(with_notes(err, notes, help));
            }
        };
        let note = |e: RuntimeError| with_notes(e, method_note(method_name, &method), None);
        if method.params.iter().any(|p| p.is_self) {
            return Err(note(RuntimeError::MethodNeedsInstance {
                class: def.name.clone(),
                member: method_name.to_string(),
            }));
        }
        if method.body.is_none() {
            return Err(note(RuntimeError::UnregisteredBuiltin {
                target: format!("{}::{}", def.name, method_name),
            }));
        }
        let body = method.body.as_ref().expect("body checked above");
        if args.len() != method.params.len() {
            return Err(note(RuntimeError::Arity {
                callable: format!("{}::{}", def.name, method_name),
                expected: crate::error::Arity::Exact(method.params.len()),
                got: args.len(),
            }));
        }
        let call_env = Env::child(&method.env);
        for (p, a) in method.params.iter().zip(args) {
            call_env.borrow_mut().set_value(&p.name.value, a);
        }
        self.eval_block_tail(&call_env, body).map_err(note)
    }

    /// Evaluate a method call `obj.method(args)` (spec §4.5), including the builtin `String` methods (spec §18.1).
    pub(crate) fn eval_method_call(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &Spanned<String>,
        args: &[Expr],
    ) -> Result<Value, RuntimeError> {
        let rcv = self.eval_expr(env, receiver)?;
        let mut arg_values = Vec::with_capacity(args.len());
        for a in args {
            arg_values.push(self.eval_expr(env, a)?);
        }
        match rcv {
            Value::Class(id) => {
                let inst = self
                    .instances
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| RuntimeError::UnknownClassInstance)?;
                let def = self.class_defs.get(&inst.class).cloned().ok_or_else(|| {
                    RuntimeError::UnknownClass {
                        name: inst.class.clone(),
                    }
                })?;
                let method = match def.methods.get(&name.value) {
                    Some(m) => m.clone(),
                    None => {
                        // Unknown method: attach a `did you mean` help and a doc note from the
                        // nearest candidate method (spec §16.4).
                        let err = RuntimeError::UnknownMethod {
                            name: name.value.clone(),
                            class: def.name.clone(),
                        };
                        let mut notes = Vec::new();
                        let mut help = None;
                        if let Some(cand) =
                            did_you_mean(&name.value, def.methods.keys().map(|k| k.as_str()))
                        {
                            if cand != name.value {
                                help = Some(format!("did you mean `{cand}`?"));
                            }
                            if let Some(m) = def.methods.get(&cand) {
                                notes.extend(method_note(&cand, m));
                            }
                        }
                        return Err(with_notes(err, notes, help));
                    }
                };
                if method.vis == Visibility::Private && !self.in_method_of(&def.name) {
                    return Err(with_notes(
                        RuntimeError::PrivateMethod {
                            name: name.value.clone(),
                        },
                        method_note(&name.value, &method),
                        None,
                    ));
                }
                if method.vis == Visibility::Module && self.current_module != def.module {
                    return Err(with_notes(
                        RuntimeError::ModuleOnlyMethod {
                            name: name.value.clone(),
                            class: def.name.clone(),
                        },
                        method_note(&name.value, &method),
                        None,
                    ));
                }
                if !method.params.first().map(|p| p.is_self).unwrap_or(false) {
                    return Err(with_notes(
                        RuntimeError::AssociatedFunctionCalled {
                            class: def.name.clone(),
                            member: name.value.clone(),
                        },
                        method_note(&name.value, &method),
                        None,
                    ));
                }
                self.call_method(&method, Value::Class(id), arg_values)
                    .map_err(|e| with_notes(e, method_note(&name.value, &method), None))
            }
            // Builtin classes (spec §18.1): the method is looked up in the embedded `core::<class>`
            // module and dispatched through its registered impl / `.pra` body / runtime backend.
            Value::String(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "String",
                rcv,
                &name.value,
                arg_values,
                None,
            ),
            Value::Number(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "Number",
                rcv,
                &name.value,
                arg_values,
                Some(BuiltinBackend::CollapseNumber),
            ),
            Value::Array(_) => {
                let backend = if is_mutating_array_method(&name.value) {
                    BuiltinBackend::MutateArray
                } else {
                    BuiltinBackend::Array
                };
                self.dispatch_builtin_method(
                    env,
                    receiver,
                    "Array",
                    rcv,
                    &name.value,
                    arg_values,
                    Some(backend),
                )
            }
            Value::Dict(_) => {
                let backend = if is_mutating_dict_method(&name.value) {
                    BuiltinBackend::MutateDict
                } else {
                    BuiltinBackend::Dict
                };
                self.dispatch_builtin_method(
                    env,
                    receiver,
                    "Dict",
                    rcv,
                    &name.value,
                    arg_values,
                    Some(backend),
                )
            }
            Value::Set(_) => {
                let backend = if is_mutating_set_method(&name.value) {
                    BuiltinBackend::MutateSet
                } else {
                    BuiltinBackend::Set
                };
                self.dispatch_builtin_method(
                    env,
                    receiver,
                    "Set",
                    rcv,
                    &name.value,
                    arg_values,
                    Some(backend),
                )
            }
            Value::Char(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "Char",
                rcv,
                &name.value,
                arg_values,
                Some(BuiltinBackend::Char),
            ),
            Value::Tuple(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "Tuple",
                rcv,
                &name.value,
                arg_values,
                Some(BuiltinBackend::Tuple),
            ),
            Value::Option(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "Option",
                rcv,
                &name.value,
                arg_values,
                Some(BuiltinBackend::Collapse),
            ),
            Value::Result(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "Result",
                rcv,
                &name.value,
                arg_values,
                Some(BuiltinBackend::Collapse),
            ),
            other => crate::error::type_err(format!(
                "cannot call method `{}` on {}",
                name.value,
                value_type_name(&other)
            )),
        }
    }

    /// Call a method: `self` is bound to the receiver (a shallow copy — same instance handle, spec §12.3).
    pub(crate) fn call_method(
        &mut self,
        method: &MethodDef,
        receiver: Value,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        crate::error::catch_propagate(self.call_method_inner(method, receiver, args))
    }

    fn call_method_inner(
        &mut self,
        method: &MethodDef,
        receiver: Value,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let self_param = method
            .params
            .first()
            .filter(|p| p.is_self)
            .ok_or_else(|| RuntimeError::MissingSelfReceiver)?;
        let expected = method.params.len() - 1;
        if args.len() != expected {
            return crate::error::type_err(format!(
                "method expects {expected} arguments, got {}",
                args.len()
            ));
        }
        // Layered `@builtin(ON)` method (spec §18.4): the registered Rust implementation is used when
        // the active `opt_level` is at least the declared tier; otherwise the `.pra` body is evaluated.
        if let Some(native) = method.native
            && (method.level == 0 || self.current_config().opt_level.tier() >= method.level)
        {
            let mut full = Vec::with_capacity(args.len() + 1);
            full.push(receiver);
            full.extend(args);
            return native(self, &full);
        }
        let body = method.body.as_ref().ok_or_else(|| RuntimeError::Internal {
            problem: crate::error::InternalProblem::UnregisteredBuiltin,
        })?;
        let instance_id = match &receiver {
            Value::Class(id) => Some(*id),
            _ => None,
        };
        // Bytecode VM fast path (spec §19.5): `self` is an ordinary first parameter and field reads
        // use `Op::Field`. The receiver stack/`self` value stays active for the whole VM run, so
        // private-field access and builtin-class `self` semantics match the AST path; a body outside
        // the compiled subset falls back to `eval_block_tail` below.
        if self.current_config().vm {
            let mut full = Vec::with_capacity(args.len() + 1);
            full.push(receiver.clone());
            full.extend(args.iter().cloned());
            if let Some(id) = instance_id {
                self.self_stack.push(id);
            } else {
                self.self_values.push(receiver.clone());
            }
            let outcome = self.try_vm_single(&method.params, body, &method.env, &method.vm, full);
            if instance_id.is_some() {
                self.self_stack.pop();
            } else {
                self.self_values.pop();
            }
            if let crate::vm::exec::VmOutcome::Hit(v) = outcome? {
                return Ok(v);
            }
        }
        if let Some(id) = instance_id {
            self.self_stack.push(id);
        } else {
            // Builtin-class receiver (`Value::String`/`Array`/...): expose it as `self` in the body.
            self.self_values.push(receiver.clone());
        }
        let call_env = Env::child(&method.env);
        {
            let mut e = call_env.borrow_mut();
            e.set_value(&self_param.name.value, receiver.clone());
            for (p, a) in method.params.iter().skip(1).zip(args) {
                e.set_value(&p.name.value, a);
            }
        }
        let result = self.eval_block_tail(&call_env, body);
        if instance_id.is_some() {
            self.self_stack.pop();
        } else {
            self.self_values.pop();
        }
        result
    }

    /// Whether the currently executing method belongs to `class` (used for private-field access, spec §15.2).
    pub(crate) fn in_method_of(&self, class: &str) -> bool {
        self.self_stack
            .last()
            .and_then(|id| self.instances.get(id))
            .map(|i| i.class == class)
            .unwrap_or(false)
    }

    /// Whether a field is accessible from the current context (spec §15.2): private fields are readable
    /// only inside methods of the same class; `pub(mod)` fields are readable only inside the defining module.
    pub(crate) fn field_accessible(&self, def: &ClassDef, field: &FieldDef) -> bool {
        match field.vis {
            Visibility::Public => true,
            Visibility::Module => self.current_module == def.module,
            Visibility::Private => self.in_method_of(&def.name),
        }
    }

    /// Field access `obj.field` (spec §4.5): private fields are readable only inside methods of the same class.
    pub(crate) fn eval_field(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &Spanned<String>,
    ) -> Result<Value, RuntimeError> {
        let rcv = self.eval_expr(env, receiver)?;
        self.field_value(rcv, &name.value)
    }

    /// Read `receiver.field` from an already-evaluated receiver (spec §4.5). Shared by the AST
    /// `eval_field` path and the bytecode VM's `Op::Field`.
    pub(crate) fn field_value(&self, rcv: Value, name: &str) -> Result<Value, RuntimeError> {
        match rcv {
            Value::Class(id) => {
                let inst = self
                    .instances
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| RuntimeError::UnknownClassInstance)?;
                let def = self.class_defs.get(&inst.class).cloned().ok_or_else(|| {
                    RuntimeError::UnknownClass {
                        name: inst.class.clone(),
                    }
                })?;
                let field =
                    def.fields
                        .get(name)
                        .cloned()
                        .ok_or_else(|| RuntimeError::UnknownField {
                            class: def.name.clone(),
                            name: name.to_string(),
                        })?;
                if !self.field_accessible(&def, &field) {
                    return crate::error::type_err(format!(
                        "field `{}` of `{}` is not accessible here",
                        name, def.name
                    ));
                }
                inst.fields
                    .get(name)
                    .cloned()
                    .ok_or_else(|| RuntimeError::UninitializedField {
                        field: name.to_string(),
                    })
            }
            other => crate::error::type_err(format!(
                "field access requires a class instance, got {}",
                value_type_name(&other)
            )),
        }
    }

    /// Struct literal `T { a, b, ..base }` (spec §4.5): unknown field `E0060`, missing field `E0061`.
    pub(crate) fn eval_struct_literal(
        &mut self,
        env: &EnvRef,
        name: &Spanned<String>,
        fields: &[FieldValue],
        base: Option<&Expr>,
    ) -> Result<Value, RuntimeError> {
        let def = self.class_defs.get(&name.value).cloned().ok_or_else(|| {
            RuntimeError::UnknownClass {
                name: name.value.clone(),
            }
        })?;
        let mut provided: HashSet<String> = HashSet::new();
        let mut out_fields: HashMap<String, Value> = HashMap::new();
        for fv in fields {
            if !def.fields.contains_key(&fv.name.value) {
                let err = RuntimeError::CompileDiagnostic {
                    code: "E0060",
                    message: format!(
                        "unknown field `{}` in `{}` literal",
                        fv.name.value, name.value
                    ),
                };
                // Suggest the nearest declared field (spec §16.4).
                let help = did_you_mean_help(&fv.name.value, def.fields.keys());
                return Err(match help {
                    Some(h) => err.with_help(h),
                    None => err,
                });
            }
            let v = match &fv.value {
                Some(e) => self.eval_expr(env, e)?,
                None => env.borrow().get_value(&fv.name.value).ok_or_else(|| {
                    RuntimeError::UnknownVariable {
                        name: fv.name.value.clone(),
                    }
                })?,
            };
            provided.insert(fv.name.value.clone());
            out_fields.insert(fv.name.value.clone(), v);
        }
        if let Some(b) = base {
            match self.eval_expr(env, b)? {
                Value::Class(bid) => {
                    let binst = self
                        .instances
                        .get(&bid)
                        .cloned()
                        .ok_or_else(|| RuntimeError::UnknownClassInstance)?;
                    if binst.class != def.name {
                        return crate::error::type_err(format!(
                            "`{}` literal base must be a `{}` instance",
                            name.value, def.name
                        ));
                    }
                    for (k, v) in binst.fields {
                        if !provided.contains(&k) {
                            out_fields.insert(k, v);
                        }
                    }
                }
                _ => {
                    return crate::error::type_err("struct literal base must be a class instance");
                }
            }
        }
        for f in def.fields.keys() {
            if !out_fields.contains_key(f) {
                return Err(RuntimeError::CompileDiagnostic {
                    code: "E0061",
                    message: format!("missing field `{f}` in `{}` literal", name.value),
                });
            }
        }
        let id = self.next_instance_id;
        self.next_instance_id += 1;
        self.instances.insert(
            id,
            ClassInstance {
                class: def.name,
                fields: out_fields,
            },
        );
        Ok(Value::Class(id))
    }

    /// Dispatch backend for a builtin-class method (spec §18.1): mutating methods write back through
    /// the receiver binding; `Number`/`Option`/`Result` go through the collapse family; everything
    /// else is a read-only backend. User classes are always `Plain`.
    pub(crate) fn method_nature(&self, class: &str, method: &str) -> MethodNature {
        match class {
            "Array" if is_mutating_array_method(method) => MethodNature::Mutating,
            "Dict" if is_mutating_dict_method(method) => MethodNature::Mutating,
            "Set" if is_mutating_set_method(method) => MethodNature::Mutating,
            "Array" | "Dict" | "Set" | "Char" | "Tuple" => MethodNature::Backend,
            "Number" | "Option" | "Result" => MethodNature::Collapse,
            _ => MethodNature::Plain,
        }
    }

    /// Build a `ClassDef` from a class statement (spec §4.5): fields and methods.
    ///
    /// A method's `@builtin(ON)` annotation (spec §18.4) drives validation: bare `@builtin` (O0) is
    /// signature-only and must be backed by a registered implementation or a runtime backend
    /// (`E0055`), with a body rejected as `E0056`; `@builtin(ON)` (`O1..O3`) requires a `.pra`
    /// fallback body (`E0056`) and an optional Rust fast path. Plain methods with neither a body nor
    /// an implementation are also `E0055`.
    pub(crate) fn build_class_def(
        &mut self,
        name: &Spanned<String>,
        members: &[prima_syntax::ast::ClassMember],
        docs: Option<&DocComment>,
        env: &EnvRef,
    ) -> Result<ClassDef, RuntimeError> {
        let mut def = ClassDef {
            name: name.value.clone(),
            module: self.current_module.clone(),
            fields: HashMap::new(),
            methods: HashMap::new(),
            docs: docs.cloned(),
        };
        for m in members {
            match &m.kind {
                ClassMemberKind::Field { name: fname, ty } => {
                    def.fields.insert(
                        fname.value.clone(),
                        FieldDef {
                            ty: ty.clone(),
                            vis: m.vis,
                        },
                    );
                }
                ClassMemberKind::Method {
                    name: mname,
                    params,
                    ret,
                    annotations,
                    body,
                } => {
                    let is_builtin = annotations.iter().any(|a| a.is_builtin());
                    let level = annotations
                        .iter()
                        .map(|a| a.builtin_level())
                        .max()
                        .unwrap_or(0);
                    let nature = self.method_nature(&def.name, &mname.value);
                    let native = if is_builtin {
                        crate::stdlib::get_impl(&format!("{}::{}", def.name, mname.value))
                    } else {
                        None
                    };
                    // Validation (spec §18.4): `E0055` unregistered `@builtin`, `E0056` wrong body.
                    if is_builtin && level == 0 && body.is_some() {
                        return Err(RuntimeError::CompileDiagnostic {
                            code: "E0056",
                            message: format!(
                                "`@builtin` method `{}` of `{}` must not have a body",
                                mname.value, def.name
                            ),
                        });
                    }
                    if is_builtin && level > 0 && body.is_none() {
                        return Err(RuntimeError::CompileDiagnostic {
                            code: "E0056",
                            message: format!(
                                "`@builtin(O{level})` method `{}` of `{}` must have a `.pra` fallback body",
                                mname.value, def.name
                            ),
                        });
                    }
                    if body.is_none() && native.is_none() && nature == MethodNature::Plain {
                        return Err(RuntimeError::CompileDiagnostic {
                            code: "E0055",
                            message: format!(
                                "unregistered `@builtin` method `{}` of `{}`",
                                mname.value, def.name
                            ),
                        });
                    }
                    def.methods.insert(
                        mname.value.clone(),
                        MethodDef {
                            params: params.clone(),
                            ret: ret.clone(),
                            body: body.clone(),
                            native,
                            level,
                            nature,
                            vis: m.vis,
                            env: Rc::clone(env),
                            vm: Default::default(),
                            docs: m.docs.clone(),
                        },
                    );
                }
            }
        }
        Ok(def)
    }

    /// Register a class in the registry (spec §4.7). A later definition with the same name wins.
    pub(crate) fn register_class(&mut self, def: ClassDef) {
        self.class_defs.insert(def.name.clone(), def);
    }

    /// Lazily load a builtin class definition (`String`/`Array`/...) from its embedded `core::<class>`
    /// module (spec §18.1). The modules are registered by the standard library; without
    /// `prima_stdlib::init()` the class is unavailable and its methods error out.
    pub(crate) fn ensure_class(&mut self, class: &str) -> Result<(), RuntimeError> {
        if self.class_defs.contains_key(class) {
            return Ok(());
        }
        let module_path = format!("core::{}", class.to_ascii_lowercase());
        let src = crate::stdlib::get_module_source(&module_path).ok_or_else(|| {
            RuntimeError::NotInitialized {
                subject: class.to_string(),
            }
        })?;
        let program = prima_syntax::parse(src).map_err(|_| RuntimeError::Internal {
            problem: crate::error::InternalProblem::EmbeddedModuleParse {
                module: module_path.clone(),
            },
        })?;
        let env = Env::new().into_ref();
        let prev_module = std::mem::take(&mut self.current_module);
        self.current_module = module_path.clone();
        let result = (|| {
            for stmt in &program.stmts {
                if let Stmt::ClassDef {
                    name,
                    members,
                    docs,
                    ..
                } = stmt
                    && name.value == class
                {
                    let def = self.build_class_def(name, members, docs.as_ref(), &env)?;
                    self.register_class(def);
                }
            }
            if self.class_defs.contains_key(class) {
                Ok(())
            } else {
                crate::error::type_err(format!(
                    "internal error: `{module_path}` does not define `class {class}`"
                ))
            }
        })();
        self.current_module = prev_module;
        result
    }
}

mod arc;
mod builtin;
mod mutation;
