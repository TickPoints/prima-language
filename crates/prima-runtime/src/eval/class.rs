//! Class and value-type dispatch (spec §4.5/§5/§12/§18): class resolution, associated functions,
//! method/field access, struct literals, and builtin dispatch for `String`/`Array`/`Dict`/`Set`/
//! `Char`/`Tuple` plus their mutating operations.

use super::helpers::{
    did_you_mean, is_mutating_array_method, is_mutating_dict_method, is_mutating_set_method,
    method_note, native_method_error, normalize_index, normalize_insert, numeric_method_name,
    path_key, with_notes,
};
use super::*;

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
        let method = match def.methods.get(method_name) {
            Some(m) => m.clone(),
            None => {
                let err = RuntimeError::Message(format!(
                    "unknown associated function `{}::{}`",
                    def.name, method_name
                ));
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
            return Err(note(RuntimeError::Message(format!(
                "`{}::{}` is a method; call it on an instance",
                def.name, method_name
            ))));
        }
        if method.body.is_none() {
            return Err(note(RuntimeError::Message(format!(
                "`{}::{}` is an unregistered `@builtin` method",
                def.name, method_name
            ))));
        }
        let body = method.body.as_ref().expect("body checked above");
        if args.len() != method.params.len() {
            return Err(note(RuntimeError::Message(format!(
                "`{}::{}` expects {} arguments, got {}",
                def.name,
                method_name,
                method.params.len(),
                args.len()
            ))));
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
                    .ok_or_else(|| RuntimeError::Message("unknown class instance".into()))?;
                let def = self.class_defs.get(&inst.class).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("unknown class `{}`", inst.class))
                })?;
                let method = match def.methods.get(&name.value) {
                    Some(m) => m.clone(),
                    None => {
                        // Unknown method: attach a `did you mean` help and a doc note from the
                        // nearest candidate method (spec §16.4).
                        let err = RuntimeError::Message(format!(
                            "unknown method `{}` on `{}`",
                            name.value, def.name
                        ));
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
                        RuntimeError::Message(format!(
                            "private method `{}` cannot be called",
                            name.value
                        )),
                        method_note(&name.value, &method),
                        None,
                    ));
                }
                if method.vis == Visibility::Module && self.current_module != def.module {
                    return Err(with_notes(
                        RuntimeError::Message(format!(
                            "method `{}` of `{}` is `pub(mod)` and not accessible from this module",
                            name.value, def.name
                        )),
                        method_note(&name.value, &method),
                        None,
                    ));
                }
                if !method.params.first().map(|p| p.is_self).unwrap_or(false) {
                    return Err(with_notes(
                        RuntimeError::Message(format!(
                            "`{}` on `{}` is an associated function; call it as `{}::{}(...)`",
                            name.value, def.name, def.name, name.value
                        )),
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
            other => crate::error::err(format!(
                "cannot call method `{}` on {}",
                name.value,
                value_type_name(&other)
            )),
        }
    }

    /// Dispatch a builtin-class method call through its class definition (spec §18.1): a registered
    /// Rust fast path or `.pra` fallback body when the method declares one (layered `@builtin(ON)`,
    /// spec §18.4), otherwise the runtime backend. Errors are wrapped with the method's doc note
    /// (spec §16.4).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch_builtin_method(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        class: &str,
        rcv: Value,
        name: &str,
        args: Vec<Value>,
        backend: Option<BuiltinBackend>,
    ) -> Result<Value, RuntimeError> {
        let Some(method) = self.builtin_method(class, name)? else {
            let err = RuntimeError::Message(format!("unknown `{class}` method `{name}`"));
            return Err(native_method_error(class, name, err));
        };
        // Registered impl and/or `.pra` body: `call_method` applies the `@builtin(ON)` layering.
        if method.body.is_some() || method.native.is_some() {
            return self
                .call_method(&method, rcv, args)
                .map_err(|e| native_method_error(class, name, e));
        }
        let backend = backend.ok_or_else(|| {
            RuntimeError::Message(format!("unregistered `{class}` method `{name}`"))
        })?;
        let result = match backend {
            BuiltinBackend::Array => {
                let Value::Array(a) = &rcv else {
                    return crate::error::err("expected an array receiver");
                };
                self.call_array_method(a, name, args)
            }
            BuiltinBackend::Dict => {
                let Value::Dict(d) = &rcv else {
                    return crate::error::err("expected a dict receiver");
                };
                self.call_dict_method(d, name, args)
            }
            BuiltinBackend::Set => {
                let Value::Set(s) = &rcv else {
                    return crate::error::err("expected a set receiver");
                };
                self.call_set_method(s, name, args)
            }
            BuiltinBackend::MutateArray => self.mutate_array(env, receiver, name, args),
            BuiltinBackend::MutateDict => self.mutate_dict(env, receiver, name, args),
            BuiltinBackend::MutateSet => self.mutate_set(env, receiver, name, args),
            BuiltinBackend::Char => {
                let Value::Char(c) = &rcv else {
                    return crate::error::err("expected a char receiver");
                };
                self.call_char_method(*c, name, args)
            }
            BuiltinBackend::Tuple => {
                let Value::Tuple(t) = &rcv else {
                    return crate::error::err("expected a tuple receiver");
                };
                self.call_tuple_method(t, name, args)
            }
            BuiltinBackend::Collapse | BuiltinBackend::CollapseNumber => {
                let collapse_name = if matches!(backend, BuiltinBackend::CollapseNumber) {
                    numeric_method_name(name)
                } else {
                    name.to_string()
                };
                let mut cargs = Vec::with_capacity(args.len() + 1);
                cargs.push(rcv);
                cargs.extend(args);
                crate::collapse::call(&collapse_name, &cargs, self.pool, self.builtins)
            }
        };
        result.map_err(|e| native_method_error(class, name, e))
    }

    /// Call a method: `self` is bound to the receiver (a shallow copy — same instance handle, spec §12.3).
    pub(crate) fn call_method(
        &mut self,
        method: &MethodDef,
        receiver: Value,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let self_param = method
            .params
            .first()
            .filter(|p| p.is_self)
            .ok_or_else(|| RuntimeError::Message("method requires a `self` receiver".into()))?;
        let expected = method.params.len() - 1;
        if args.len() != expected {
            return crate::error::err(format!(
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
        let body = method
            .body
            .as_ref()
            .ok_or_else(|| RuntimeError::Message("unregistered `@builtin` method".into()))?;
        let instance_id = match &receiver {
            Value::Class(id) => Some(*id),
            _ => None,
        };
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
        match rcv {
            Value::Class(id) => {
                let inst = self
                    .instances
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| RuntimeError::Message("unknown class instance".into()))?;
                let def = self.class_defs.get(&inst.class).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("unknown class `{}`", inst.class))
                })?;
                let field = def.fields.get(&name.value).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("`{}` has no field `{}`", def.name, name.value))
                })?;
                if !self.field_accessible(&def, &field) {
                    return crate::error::err(format!(
                        "field `{}` of `{}` is not accessible here",
                        name.value, def.name
                    ));
                }
                inst.fields.get(&name.value).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("field `{}` is uninitialized", name.value))
                })
            }
            other => crate::error::err(format!(
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
        let def = self
            .class_defs
            .get(&name.value)
            .cloned()
            .ok_or_else(|| RuntimeError::Message(format!("unknown class `{}`", name.value)))?;
        let mut provided: HashSet<String> = HashSet::new();
        let mut out_fields: HashMap<String, Value> = HashMap::new();
        for fv in fields {
            if !def.fields.contains_key(&fv.name.value) {
                return crate::error::err(format!(
                    "unknown field `{}` in `{}` literal",
                    fv.name.value, name.value
                ));
            }
            let v = match &fv.value {
                Some(e) => self.eval_expr(env, e)?,
                None => env.borrow().get_value(&fv.name.value).ok_or_else(|| {
                    RuntimeError::Message(format!(
                        "no value `{}` in scope for field shorthand",
                        fv.name.value
                    ))
                })?,
            };
            provided.insert(fv.name.value.clone());
            out_fields.insert(fv.name.value.clone(), v);
        }
        if let Some(b) = base {
            match self.eval_expr(env, b)? {
                Value::Class(bid) => {
                    let binst =
                        self.instances.get(&bid).cloned().ok_or_else(|| {
                            RuntimeError::Message("unknown class instance".into())
                        })?;
                    if binst.class != def.name {
                        return crate::error::err(format!(
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
                _ => return crate::error::err("struct literal base must be a class instance"),
            }
        }
        for f in def.fields.keys() {
            if !out_fields.contains_key(f) {
                return crate::error::err(format!(
                    "missing field `{f}` in `{}` literal",
                    name.value
                ));
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
                        return crate::error::err(format!(
                            "`@builtin` method `{}` of `{}` must not have a body (E0056)",
                            mname.value, def.name
                        ));
                    }
                    if is_builtin && level > 0 && body.is_none() {
                        return crate::error::err(format!(
                            "`@builtin(O{level})` method `{}` of `{}` must have a `.pra` fallback body (E0056)",
                            mname.value, def.name
                        ));
                    }
                    if body.is_none() && native.is_none() && nature == MethodNature::Plain {
                        return crate::error::err(format!(
                            "unregistered `@builtin` method `{}` of `{}` (E0055)",
                            mname.value, def.name
                        ));
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
            RuntimeError::Message(format!(
                "`{class}` methods are unavailable: the standard library is not initialized"
            ))
        })?;
        let program = prima_syntax::parse(src).map_err(|_| {
            RuntimeError::Message(format!(
                "internal error: embedded `{module_path}` module does not parse"
            ))
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
                crate::error::err(format!(
                    "internal error: `{module_path}` does not define `class {class}`"
                ))
            }
        })();
        self.current_module = prev_module;
        result
    }

    /// Look up a builtin-class method, loading the class definition first (spec §18.1). `None` for an
    /// unknown method; loading fails with a runtime error when the standard library is missing.
    pub(crate) fn builtin_method(
        &mut self,
        class: &str,
        method: &str,
    ) -> Result<Option<MethodDef>, RuntimeError> {
        self.ensure_class(class)?;
        Ok(self
            .class_defs
            .get(class)
            .and_then(|d| d.methods.get(method))
            .cloned())
    }

    /// Builtin `Char` methods (spec §18.1): predicates, case mapping, and conversion. All are
    /// zero-argument; `to_upper`/`to_lower` map a single char (a multi-char expansion, e.g. `ß`,
    /// keeps only the first code point).
    pub(crate) fn call_char_method(
        &mut self,
        c: char,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty() {
            return crate::error::err(format!(
                "`Char.{name}` takes no arguments, got {}",
                args.len()
            ));
        }
        match name {
            "len" => Ok(Value::Number(Number::from(1))),
            "is_digit" => Ok(Value::Bool(c.is_ascii_digit())),
            "is_alpha" => Ok(Value::Bool(c.is_alphabetic())),
            "is_alnum" => Ok(Value::Bool(c.is_alphanumeric())),
            "is_upper" => Ok(Value::Bool(c.is_uppercase())),
            "is_lower" => Ok(Value::Bool(c.is_lowercase())),
            "is_space" => Ok(Value::Bool(c.is_whitespace())),
            "is_ascii" => Ok(Value::Bool(c.is_ascii())),
            "to_upper" => Ok(Value::Char(c.to_uppercase().next().unwrap_or(c))),
            "to_lower" => Ok(Value::Char(c.to_lowercase().next().unwrap_or(c))),
            "to_string" => Ok(Value::String(c.to_string())),
            "code" => Ok(Value::Number(Number::from(u32::from(c) as i64))),
            _ => crate::error::err(format!("unknown `Char` method `{name}`")),
        }
    }

    /// Builtin `Tuple` methods (spec §18.1): `len`/`get`/`count`/`index`/`first`/`last`.
    pub(crate) fn call_tuple_method(
        &mut self,
        t: &[Value],
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
                    "`Tuple.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        match name {
            "len" => {
                arity(0)?;
                Ok(Value::Number(Number::from(t.len() as i64)))
            }
            "get" => {
                arity(1)?;
                let i = match &args[0] {
                    Value::Number(n) => n.as_i64(),
                    _ => None,
                };
                let Some(i) = i else {
                    return crate::error::err("`Tuple.get` expects an integer index");
                };
                match normalize_index(i, t.len()) {
                    Some(i) => Ok(Value::Option(Some(Box::new(t[i].clone())))),
                    None => Ok(Value::Option(None)),
                }
            }
            "count" => {
                arity(1)?;
                Ok(Value::Number(Number::from(
                    t.iter().filter(|e| self.value_eq(e, &args[0])).count() as i64,
                )))
            }
            "index" => {
                arity(1)?;
                match t.iter().position(|e| self.value_eq(e, &args[0])) {
                    Some(i) => Ok(Value::Number(Number::from(i as i64))),
                    None => crate::error::err("element not found"),
                }
            }
            "first" => {
                arity(0)?;
                Ok(t.first()
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            "last" => {
                arity(0)?;
                Ok(t.last()
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            _ => crate::error::err(format!("unknown `Tuple` method `{name}`")),
        }
    }

    /// `String::new()` / `String::from(x)` associated functions (spec §18.1).
    pub(crate) fn try_string_associated(
        &mut self,
        env: &EnvRef,
        segments: &[Spanned<String>],
        args: &[Expr],
    ) -> Result<Option<Value>, RuntimeError> {
        if segments.len() != 2 || segments[0].value != "String" {
            return Ok(None);
        }
        match segments[1].value.as_str() {
            "new" => {
                if !args.is_empty() {
                    return crate::error::err("`String::new` takes no arguments");
                }
                Ok(Some(Value::String(String::new())))
            }
            "from" => {
                if args.len() != 1 {
                    return crate::error::err("`String::from` expects 1 argument");
                }
                let v = self.eval_expr(env, &args[0])?;
                Ok(Some(Value::String(self.format_value(&v))))
            }
            _ => Ok(None),
        }
    }

    /// `get(array, index) -> Option<T>`: safe array access (spec §11.3); a negative index counts from
    /// the end, out-of-range yields `None`.
    pub(crate) fn call_array_get(&mut self, arr: Value, idx: Value) -> Result<Value, RuntimeError> {
        let Value::Array(a) = arr else {
            return crate::error::err("`get` expects an array");
        };
        let i = match idx {
            Value::Number(n) => n.as_i64(),
            _ => None,
        };
        let Some(i) = i else {
            return crate::error::err("`get` expects an integer index");
        };
        match normalize_index(i, a.len()) {
            Some(i) => Ok(Value::Option(Some(Box::new(a[i].clone())))),
            None => Ok(Value::Option(None)),
        }
    }

    /// Read-only array methods (spec §11.3): operate on a value-semantic copy of the array.
    pub(crate) fn call_array_method(
        &mut self,
        a: &[Value],
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
                    "`Array.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        match name {
            "len" => {
                arity(0)?;
                Ok(Value::Number(Number::from(a.len() as i64)))
            }
            "is_empty" => {
                arity(0)?;
                Ok(Value::Bool(a.is_empty()))
            }
            "get" => {
                arity(1)?;
                self.call_array_get(Value::Array(a.to_vec()), args[0].clone())
            }
            "contains" => {
                arity(1)?;
                Ok(Value::Bool(a.iter().any(|e| self.value_eq(e, &args[0]))))
            }
            "index" => {
                arity(1)?;
                match a.iter().position(|e| self.value_eq(e, &args[0])) {
                    Some(i) => Ok(Value::Number(Number::from(i as i64))),
                    None => crate::error::err("element not found"),
                }
            }
            "count" => {
                arity(1)?;
                Ok(Value::Number(Number::from(
                    a.iter().filter(|e| self.value_eq(e, &args[0])).count() as i64,
                )))
            }
            "first" => {
                arity(0)?;
                Ok(a.first()
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            "last" => {
                arity(0)?;
                Ok(a.last()
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            "copy" => {
                arity(0)?;
                Ok(Value::Array(a.to_vec()))
            }
            _ => crate::error::err(format!("unknown `Array` method `{name}`")),
        }
    }

    /// Mutating array methods (spec §11.3): the receiver must be a single-segment path (a variable
    /// binding); the mutated copy is written back to the binding.
    pub(crate) fn mutate_array(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let var = match &receiver.kind {
            ExprKind::Path { segments } if segments.len() == 1 => segments[0].value.clone(),
            _ => return crate::error::err("cannot mutate a temporary value"),
        };
        let cur = env
            .borrow()
            .get_value(&var)
            .ok_or_else(|| RuntimeError::Message(format!("unknown variable `{var}`")))?;
        let Value::Array(mut arr) = cur else {
            return crate::error::err("expected an array binding");
        };
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
                    "`Array.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        let index = |v: &Value| -> Result<i64, RuntimeError> {
            match v {
                Value::Number(n) => n.as_i64().ok_or_else(|| {
                    RuntimeError::Type(format!("`Array.{name}` index must be an integer"))
                }),
                _ => crate::error::err(format!("`Array.{name}` index must be an integer")),
            }
        };
        let out = match name {
            "push" => {
                arity(1)?;
                arr.push(args[0].clone());
                Value::Nil
            }
            "pop" => {
                arity(0)?;
                arr.pop()
                    .map(|v| Value::Option(Some(Box::new(v))))
                    .unwrap_or(Value::Option(None))
            }
            "append" => {
                arity(1)?;
                arr.push(args[0].clone());
                Value::Nil
            }
            "extend" => {
                arity(1)?;
                match &args[0] {
                    Value::Array(elems) => arr.extend(elems.iter().cloned()),
                    other => {
                        return crate::error::err(format!(
                            "`Array.extend` expects an array, got {}",
                            value_type_name(other)
                        ));
                    }
                }
                Value::Nil
            }
            "insert" => {
                arity(2)?;
                let i = index(&args[0])?;
                let i = normalize_insert(i, arr.len()).ok_or_else(|| {
                    RuntimeError::IndexOutOfBounds(format!("index {i} (length {})", arr.len()))
                })?;
                arr.insert(i, args[1].clone());
                Value::Nil
            }
            "remove" => {
                arity(1)?;
                let i = index(&args[0])?;
                let i = normalize_index(i, arr.len()).ok_or_else(|| {
                    RuntimeError::IndexOutOfBounds(format!("index {i} (length {})", arr.len()))
                })?;
                arr.remove(i)
            }
            "clear" => {
                arity(0)?;
                arr.clear();
                Value::Nil
            }
            // `sort` orders numeric elements only (spec §11.3); `reverse` works on any elements.
            "sort" => {
                arity(0)?;
                let mut nums = Vec::with_capacity(arr.len());
                for e in &arr {
                    match e {
                        Value::Number(n) => nums.push(n.clone()),
                        _ => return crate::error::err("`Array.sort` requires an array of numbers"),
                    }
                }
                nums.sort_by(|x, y| self.number_cmp(x, y).unwrap_or(Ordering::Equal));
                arr = nums.into_iter().map(Value::Number).collect();
                Value::Nil
            }
            "reverse" => {
                arity(0)?;
                arr.reverse();
                Value::Nil
            }
            _ => return crate::error::err(format!("unknown `Array` method `{name}`")),
        };
        self.write_back(env, &var, Value::Array(arr));
        Ok(out)
    }

    /// Read-only dict methods (spec §11.6): `keys`/`values`/`items` return arrays in deterministic
    /// (canonical-key sorted) order.
    pub(crate) fn call_dict_method(
        &mut self,
        d: &HashMap<ValueKey, Value>,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
                    "`Dict.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        match name {
            "len" => {
                arity(0)?;
                Ok(Value::Number(Number::from(d.len() as i64)))
            }
            "get" => {
                arity(1)?;
                let key = ValueKey::from_value(&args[0]).ok_or_else(|| {
                    RuntimeError::Message("dict key must be a hashable value".into())
                })?;
                Ok(d.get(&key)
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            "contains" => {
                arity(1)?;
                let key = ValueKey::from_value(&args[0]).ok_or_else(|| {
                    RuntimeError::Message("dict key must be a hashable value".into())
                })?;
                Ok(Value::Bool(d.contains_key(&key)))
            }
            "keys" => {
                arity(0)?;
                Ok(Value::Array(
                    self.sorted_dict_keys(d)
                        .iter()
                        .map(|k| k.to_value())
                        .collect(),
                ))
            }
            "values" => {
                arity(0)?;
                Ok(Value::Array(
                    self.sorted_dict_keys(d)
                        .iter()
                        .map(|k| d[k].clone())
                        .collect(),
                ))
            }
            "items" => {
                arity(0)?;
                Ok(Value::Array(
                    self.sorted_dict_keys(d)
                        .iter()
                        .map(|k| Value::Tuple(vec![k.to_value(), d[k].clone()]))
                        .collect(),
                ))
            }
            _ => crate::error::err(format!("unknown `Dict` method `{name}`")),
        }
    }

    /// Mutating dict methods (spec §11.6): receiver must be a single-segment path; write-back pattern
    /// mirrors `mutate_array`.
    pub(crate) fn mutate_dict(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let var = match &receiver.kind {
            ExprKind::Path { segments } if segments.len() == 1 => segments[0].value.clone(),
            _ => return crate::error::err("cannot mutate a temporary value"),
        };
        let cur = env
            .borrow()
            .get_value(&var)
            .ok_or_else(|| RuntimeError::Message(format!("unknown variable `{var}`")))?;
        let Value::Dict(mut d) = cur else {
            return crate::error::err("expected a dict binding");
        };
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
                    "`Dict.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        let key = |i: usize| -> Result<ValueKey, RuntimeError> {
            args.get(i)
                .and_then(ValueKey::from_value)
                .ok_or_else(|| RuntimeError::Message("dict key must be a hashable value".into()))
        };
        let out = match name {
            "insert" => {
                arity(2)?;
                d.insert(key(0)?, args[1].clone());
                Value::Nil
            }
            "remove" => {
                arity(1)?;
                d.remove(&key(0)?)
                    .map(|v| Value::Option(Some(Box::new(v))))
                    .unwrap_or(Value::Option(None))
            }
            "clear" => {
                arity(0)?;
                d.clear();
                Value::Nil
            }
            "update" => {
                arity(1)?;
                let Value::Dict(other) = &args[0] else {
                    return crate::error::err("`Dict.update` expects a dict argument");
                };
                for (k, v) in other {
                    d.insert(k.clone(), v.clone());
                }
                // `d.update(other)` returns the merged dict (spec §11.6 example `let dd = d.update(…)`).
                Value::Dict(d.clone())
            }
            "setdefault" => {
                // Python `dict.setdefault`: return the value for `key`, inserting `default` when absent.
                arity(2)?;
                let k = key(0)?;
                if let Some(v) = d.get(&k).cloned() {
                    v
                } else {
                    let v = args[1].clone();
                    d.insert(k, v.clone());
                    v
                }
            }
            "popitem" => {
                arity(0)?;
                let k = self
                    .sorted_dict_keys(&d)
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        RuntimeError::Message("`Dict.popitem` on an empty dict".into())
                    })?;
                let v = d.remove(&k).unwrap();
                Value::Tuple(vec![k.to_value(), v])
            }
            _ => return crate::error::err(format!("unknown `Dict` method `{name}`")),
        };
        self.write_back(env, &var, Value::Dict(d));
        Ok(out)
    }

    /// Read-only set methods (spec §11.6): `union`/`intersection`/`difference` return new sets.
    pub(crate) fn call_set_method(
        &mut self,
        s: &HashSet<ValueKey>,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
                    "`Set.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        match name {
            "len" => {
                arity(0)?;
                Ok(Value::Number(Number::from(s.len() as i64)))
            }
            "contains" => {
                arity(1)?;
                let key = ValueKey::from_value(&args[0]).ok_or_else(|| {
                    RuntimeError::Message("set element must be a hashable value".into())
                })?;
                Ok(Value::Bool(s.contains(&key)))
            }
            "union" | "intersection" | "difference" | "symmetric_difference" => {
                arity(1)?;
                let Value::Set(other) = &args[0] else {
                    return crate::error::err("`Set.{name}` expects a set argument");
                };
                let out = match name {
                    "union" => s.union(other).cloned().collect(),
                    "intersection" => s.intersection(other).cloned().collect(),
                    "difference" => s.difference(other).cloned().collect(),
                    "symmetric_difference" => s.symmetric_difference(other).cloned().collect(),
                    _ => unreachable!(),
                };
                Ok(Value::Set(out))
            }
            "issubset" | "issuperset" | "isdisjoint" => {
                arity(1)?;
                let Value::Set(other) = &args[0] else {
                    return crate::error::err("`Set.{name}` expects a set argument");
                };
                let out = match name {
                    "issubset" => s.is_subset(other),
                    "issuperset" => s.is_superset(other),
                    "isdisjoint" => s.is_disjoint(other),
                    _ => unreachable!(),
                };
                Ok(Value::Bool(out))
            }
            "copy" => {
                arity(0)?;
                Ok(Value::Set(s.clone()))
            }
            _ => crate::error::err(format!("unknown `Set` method `{name}`")),
        }
    }

    /// Mutating set methods (spec §11.6): `remove` reports `R0013` on an absent element, `discard` is silent.
    pub(crate) fn mutate_set(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let var = match &receiver.kind {
            ExprKind::Path { segments } if segments.len() == 1 => segments[0].value.clone(),
            _ => return crate::error::err("cannot mutate a temporary value"),
        };
        let cur = env
            .borrow()
            .get_value(&var)
            .ok_or_else(|| RuntimeError::Message(format!("unknown variable `{var}`")))?;
        let Value::Set(mut s) = cur else {
            return crate::error::err("expected a set binding");
        };
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
                    "`Set.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        let key = |i: usize| -> Result<ValueKey, RuntimeError> {
            args.get(i)
                .and_then(ValueKey::from_value)
                .ok_or_else(|| RuntimeError::Message("set element must be a hashable value".into()))
        };
        let out = match name {
            "add" => {
                arity(1)?;
                s.insert(key(0)?);
                Value::Nil
            }
            "remove" => {
                arity(1)?;
                if !s.remove(&key(0)?) {
                    return crate::error::err("element not found");
                }
                Value::Nil
            }
            "discard" => {
                arity(1)?;
                s.remove(&key(0)?);
                Value::Nil
            }
            "pop" => {
                arity(0)?;
                if let Some(k) = s.iter().next().cloned() {
                    s.remove(&k);
                    Value::Option(Some(Box::new(k.to_value())))
                } else {
                    Value::Option(None)
                }
            }
            "clear" => {
                arity(0)?;
                s.clear();
                Value::Nil
            }
            "update" => {
                arity(1)?;
                match &args[0] {
                    Value::Set(other) => {
                        for k in other {
                            s.insert(k.clone());
                        }
                    }
                    Value::Array(elems) => {
                        for e in elems {
                            let k = ValueKey::from_value(e).ok_or_else(|| {
                                RuntimeError::Message("set element must be a hashable value".into())
                            })?;
                            s.insert(k);
                        }
                    }
                    other => {
                        return crate::error::err(format!(
                            "`Set.update` expects a set or array, got {}",
                            value_type_name(other)
                        ));
                    }
                }
                Value::Nil
            }
            _ => return crate::error::err(format!("unknown `Set` method `{name}`")),
        };
        self.write_back(env, &var, Value::Set(s));
        Ok(out)
    }

    /// Deterministic key order for a dict (spec §11.6): sorted by the `format_value` of each key, with
    /// the key's debug rendering as a tiebreaker, so snapshots/tests are stable.
    pub(crate) fn sorted_dict_keys(&self, d: &HashMap<ValueKey, Value>) -> Vec<ValueKey> {
        let mut keys: Vec<ValueKey> = d.keys().cloned().collect();
        keys.sort_by(|a, b| {
            let ka = self.format_value(&a.to_value());
            let kb = self.format_value(&b.to_value());
            ka.cmp(&kb)
                .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
        });
        keys
    }

    /// Deterministic element order for a set, sorted by `format_value` (spec §11.6).
    pub(crate) fn sorted_set_values(&self, s: &HashSet<ValueKey>) -> Vec<Value> {
        let mut elems: Vec<Value> = s.iter().map(|k| k.to_value()).collect();
        elems.sort_by(|a, b| {
            let ka = self.format_value(a);
            let kb = self.format_value(b);
            ka.cmp(&kb)
                .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
        });
        elems
    }

    /// Structural dict equality (spec §11.6): same keys with promotion-equal values.
    pub(crate) fn dict_eq(
        &self,
        x: &HashMap<ValueKey, Value>,
        y: &HashMap<ValueKey, Value>,
    ) -> bool {
        if x.len() != y.len() {
            return false;
        }
        for (k, v) in x {
            match y.get(k) {
                Some(w) if self.value_eq(v, w) => {}
                _ => return false,
            }
        }
        true
    }
}
