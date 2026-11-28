//! Expression lowering for the bytecode compiler.

use super::*;

impl Compiler {
    /// Compile `expr` directly into local slot `dst` (the register channel, spec §19.5). Shapes
    /// with a register form — arithmetic over locals/immediates, `to_f64`, negation, indexing,
    /// plain moves — emit register instructions with no operand-stack traffic; everything else
    /// falls back to the stack form plus `SetLocalNc`. `dst` may alias an operand slot (the
    /// executor copies operands before writing).
    pub(super) fn compile_expr_into(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        expr: &Expr,
        dst: u16,
    ) -> Result<(), String> {
        // `dst = local`.
        if let ExprKind::Path { segments } = &expr.kind
            && segments.len() == 1
            && let Some(src) = scope.slot_of(&segments[0].value)
        {
            chunk.emit(Op::RegMove { dst, a: src }, expr.span.start);
            return Ok(());
        }
        // `dst = -local`.
        if let ExprKind::Unary {
            op: UnOp::Neg,
            operand,
        } = &expr.kind
            && let Some(a) = local_slot(scope, operand)
        {
            chunk.emit(Op::RegNeg { dst, a }, expr.span.start);
            return Ok(());
        }
        // `dst = to_f64(expr)`.
        if let ExprKind::Call { callee, args } = &expr.kind
            && let ExprKind::Path { segments } = &callee.kind
            && segments.len() == 1
            && segments[0].value == "to_f64"
            && args.len() == 1
        {
            let (a, temp) = self.operand_reg(chunk, scope, &args[0])?;
            chunk.emit(Op::RegToF64 { dst, a }, expr.span.start);
            if temp {
                scope.free_temp(a);
            }
            return Ok(());
        }
        // `dst = base[idx]` with local base and index.
        if let ExprKind::Index { base, index } = &expr.kind
            && index.items.len() == 1
            && let IndexItem::Elem(e) = &index.items[0]
            && let Some(arr) = local_slot(scope, base)
            && let Some(idx) = local_slot(scope, e)
        {
            chunk.emit(Op::RegIndex { dst, arr, idx }, expr.span.start);
            return Ok(());
        }
        // `dst = dst + l * r`: fused multiply-accumulate (spec §10). `l`/`r` may be locals or
        // array indices, which are materialized into temporaries first.
        if let ExprKind::Binary {
            op: BinOp::Add,
            lhs,
            rhs,
        } = &expr.kind
            && local_slot(scope, lhs) == Some(dst)
            && let ExprKind::Binary {
                op: BinOp::Mul,
                lhs: m1,
                rhs: m2,
            } = &rhs.kind
        {
            let (a, ta) = self.operand_reg(chunk, scope, m1)?;
            let (b, tb) = self.operand_reg(chunk, scope, m2)?;
            chunk.emit(Op::RegMulAdd { dst, a, b }, expr.span.start);
            if tb {
                scope.free_temp(b);
            }
            if ta {
                scope.free_temp(a);
            }
            return Ok(());
        }
        // Binary arithmetic over locals/immediates: combine directly, computing a complex side into
        // a temporary register first.
        if let ExprKind::Binary { op, lhs, rhs } = &expr.kind
            && let Some(aop) = arith_op(op)
        {
            if let (Some(a), Some(b)) = (local_slot(scope, lhs), local_slot(scope, rhs)) {
                chunk.emit(Op::RegBin { op: aop, dst, a, b }, expr.span.start);
                return Ok(());
            }
            if let (Some(a), Some(imm)) = (local_slot(scope, lhs), literal_imm(rhs)) {
                chunk.emit(
                    Op::RegBinImm {
                        op: aop,
                        dst,
                        a,
                        imm,
                    },
                    expr.span.start,
                );
                return Ok(());
            }
            if matches!(aop, ArithOp::Add | ArithOp::Mul)
                && let (Some(imm), Some(b)) = (literal_imm(lhs), local_slot(scope, rhs))
            {
                chunk.emit(
                    Op::RegBinImm {
                        op: aop,
                        dst,
                        a: b,
                        imm,
                    },
                    expr.span.start,
                );
                return Ok(());
            }
            // One side is complex: materialize it into a temporary, then combine.
            if let Some(b) = local_slot(scope, rhs) {
                let (a, ta) = self.operand_reg(chunk, scope, lhs)?;
                chunk.emit(Op::RegBin { op: aop, dst, a, b }, expr.span.start);
                if ta {
                    scope.free_temp(a);
                }
                return Ok(());
            }
            if let Some(imm) = literal_imm(rhs) {
                let (a, ta) = self.operand_reg(chunk, scope, lhs)?;
                chunk.emit(
                    Op::RegBinImm {
                        op: aop,
                        dst,
                        a,
                        imm,
                    },
                    expr.span.start,
                );
                if ta {
                    scope.free_temp(a);
                }
                return Ok(());
            }
            if let Some(a) = local_slot(scope, lhs) {
                let (b, tb) = self.operand_reg(chunk, scope, rhs)?;
                chunk.emit(Op::RegBin { op: aop, dst, a, b }, expr.span.start);
                if tb {
                    scope.free_temp(b);
                }
                return Ok(());
            }
            if matches!(aop, ArithOp::Add | ArithOp::Mul)
                && let Some(imm) = literal_imm(lhs)
            {
                let (b, tb) = self.operand_reg(chunk, scope, rhs)?;
                chunk.emit(
                    Op::RegBinImm {
                        op: aop,
                        dst,
                        a: b,
                        imm,
                    },
                    expr.span.start,
                );
                if tb {
                    scope.free_temp(b);
                }
                return Ok(());
            }
        }
        // Fallback: stack form + store.
        self.compile_expr(chunk, scope, expr)?;
        chunk.emit(Op::SetLocalNc(dst), expr.span.start);
        Ok(())
    }

    /// Materialize a simple operand into a register, returning `(reg, is_temp)`. A local slot is
    /// used directly (`is_temp = false`); an array index of locals emits `RegIndex` into a fresh
    /// temporary; anything else is compiled into a temporary via [`Self::compile_expr_into`].
    pub(super) fn operand_reg(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        expr: &Expr,
    ) -> Result<(u16, bool), String> {
        if let Some(s) = local_slot(scope, expr) {
            return Ok((s, false));
        }
        if let ExprKind::Index { base, index } = &expr.kind
            && index.items.len() == 1
            && let IndexItem::Elem(e) = &index.items[0]
            && let Some(arr) = local_slot(scope, base)
            && let Some(idx) = local_slot(scope, e)
        {
            let t = scope.alloc_temp(chunk)?;
            chunk.emit(Op::RegIndex { dst: t, arr, idx }, expr.span.start);
            return Ok((t, true));
        }
        let t = scope.alloc_temp(chunk)?;
        self.compile_expr_into(chunk, scope, expr, t)?;
        Ok((t, true))
    }

    pub(super) fn compile_expr(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        expr: &Expr,
    ) -> Result<(), String> {
        match &expr.kind {
            ExprKind::Literal(lit) => {
                let c = compile_literal(chunk, lit)?;
                chunk.emit(Op::Const(c), expr.span.start);
                Ok(())
            }
            ExprKind::Path { segments }
                if segments.len() == 1 && is_bool_literal(&segments[0].value) =>
            {
                let c = chunk.add_value(prima_core::Value::Bool(segments[0].value == "true"))?;
                chunk.emit(Op::Const(c), expr.span.start);
                Ok(())
            }
            ExprKind::Path { segments } if segments.len() == 1 => {
                let name = &segments[0].value;
                if let Some(slot) = scope.slot_of(name) {
                    chunk.emit(Op::LoadLocal(slot), expr.span.start);
                } else {
                    let idx = chunk.add_name(name.clone())?;
                    chunk.emit(Op::LoadName(idx), expr.span.start);
                }
                Ok(())
            }
            ExprKind::Path { .. } => Err("VM compiler: multi-segment path unsupported".into()),
            ExprKind::Binary { op, lhs, rhs } => {
                self.compile_expr(chunk, scope, lhs)?;
                self.compile_expr(chunk, scope, rhs)?;
                chunk.emit(binary_op(op)?, expr.span.start);
                Ok(())
            }
            ExprKind::Unary { op, operand } => {
                self.compile_expr(chunk, scope, operand)?;
                chunk.emit(unary_op(*op), expr.span.start);
                Ok(())
            }
            ExprKind::Self_ => {
                // `self` is bound as the method's first parameter (spec §4.5).
                if let Some(slot) = scope.slot_of("self") {
                    chunk.emit(Op::LoadLocal(slot), expr.span.start);
                } else {
                    let idx = chunk.add_name("self".to_string())?;
                    chunk.emit(Op::LoadName(idx), expr.span.start);
                }
                Ok(())
            }
            ExprKind::Field { receiver, name } => {
                self.compile_expr(chunk, scope, receiver)?;
                let idx = chunk.add_name(name.value.clone())?;
                chunk.emit(Op::Field { name: idx }, expr.span.start);
                Ok(())
            }
            ExprKind::Call { callee, args } => {
                self.compile_call(chunk, scope, callee, args, expr.span.start)
            }
            ExprKind::MethodCall {
                receiver,
                name,
                args,
            } => {
                // A mutating `Array` method (spec §11.3) on a local slot mutates the slot's array
                // in place; on an environment binding it mutates the binding's array along the
                // scope chain; any other receiver shape falls back to the AST.
                if crate::eval::is_mutating_array_method(&name.value)
                    || crate::eval::is_mutating_dict_method(&name.value)
                    || crate::eval::is_mutating_set_method(&name.value)
                {
                    let ExprKind::Path { segments } = &receiver.kind else {
                        return Err(
                            "VM compiler: mutating method receiver must be a variable".into()
                        );
                    };
                    if segments.len() != 1 {
                        return Err(
                            "VM compiler: mutating method receiver must be a variable".into()
                        );
                    }
                    for a in args {
                        self.compile_expr(chunk, scope, a)?;
                    }
                    let idx = chunk.add_name(name.value.clone())?;
                    let argc = checked_u16(args.len(), "method arguments")?;
                    match scope.slot_of(&segments[0].value) {
                        Some(slot) => {
                            chunk.emit(
                                Op::MethodLocal {
                                    name: idx,
                                    argc,
                                    slot,
                                },
                                expr.span.start,
                            );
                        }
                        None => {
                            let receiver = chunk.add_name(segments[0].value.clone())?;
                            chunk.emit(
                                Op::MethodName {
                                    receiver,
                                    name: idx,
                                    argc,
                                },
                                expr.span.start,
                            );
                        }
                    }
                    return Ok(());
                }
                self.compile_expr(chunk, scope, receiver)?;
                for a in args {
                    self.compile_expr(chunk, scope, a)?;
                }
                let idx = chunk.add_name(name.value.clone())?;
                let argc = checked_u16(args.len(), "method arguments")?;
                chunk.emit(Op::Method { name: idx, argc }, expr.span.start);
                Ok(())
            }
            ExprKind::Array(items) => {
                for it in items {
                    self.compile_expr(chunk, scope, it)?;
                }
                let n = checked_u16(items.len(), "array elements")?;
                chunk.emit(Op::MakeArray(n), expr.span.start);
                Ok(())
            }
            ExprKind::Tuple(items) => {
                for it in items {
                    self.compile_expr(chunk, scope, it)?;
                }
                let n = checked_u16(items.len(), "tuple elements")?;
                chunk.emit(Op::MakeTuple(n), expr.span.start);
                Ok(())
            }
            ExprKind::Dict(pairs) => {
                for (k, v) in pairs {
                    self.compile_expr(chunk, scope, k)?;
                    self.compile_expr(chunk, scope, v)?;
                }
                let n = checked_u16(pairs.len(), "dict entries")?;
                chunk.emit(Op::MakeDict(n), expr.span.start);
                Ok(())
            }
            ExprKind::Set(items) => {
                for it in items {
                    self.compile_expr(chunk, scope, it)?;
                }
                let n = checked_u16(items.len(), "set elements")?;
                chunk.emit(Op::MakeSet(n), expr.span.start);
                Ok(())
            }
            ExprKind::Index { base, index } => {
                // Condition/expression reads with a local base and a single element index push the
                // element directly (no receiver-handle clone): `if prime[i]` (spec §11.3).
                if let IndexItem::Elem(e) = &index.items[0]
                    && index.items.len() == 1
                    && let (Some(arr), Some(idx)) = (local_slot(scope, base), local_slot(scope, e))
                {
                    chunk.emit(Op::RegIndexPush { arr, idx }, expr.span.start);
                    return Ok(());
                }
                self.compile_expr(chunk, scope, base)?;
                if index.items.len() == 1
                    && let IndexItem::Elem(e) = &index.items[0]
                {
                    self.compile_expr(chunk, scope, e)?;
                    chunk.emit(Op::Index, expr.span.start);
                    return Ok(());
                }
                // Slices and multi-dimensional selections (spec §11.3): push one operand per element
                // index and two per slice (`Nil` for an omitted bound), then let the executor perform
                // the nested selection.
                let mut shapes = Vec::with_capacity(index.items.len());
                for item in &index.items {
                    match item {
                        IndexItem::Elem(e) => {
                            self.compile_expr(chunk, scope, e)?;
                            shapes.push(false);
                        }
                        IndexItem::Slice { start, end } => {
                            self.compile_slice_bound(chunk, scope, start.as_ref())?;
                            self.compile_slice_bound(chunk, scope, end.as_ref())?;
                            shapes.push(true);
                        }
                    }
                }
                let spec = chunk.add_index_spec(shapes);
                chunk.emit(Op::IndexMulti { spec }, expr.span.start);
                Ok(())
            }
            _ => Err("VM compiler: expression unsupported".into()),
        }
    }

    /// Push a slice bound (or the `Nil` sentinel for an omitted bound) for `IndexMulti`.
    pub(super) fn compile_slice_bound(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        bound: Option<&Expr>,
    ) -> Result<(), String> {
        match bound {
            Some(e) => self.compile_expr(chunk, scope, e),
            None => {
                let n = chunk.add_value(prima_core::Value::Nil)?;
                chunk.emit(Op::Const(n), 0);
                Ok(())
            }
        }
    }

    pub(super) fn compile_call(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        callee: &Expr,
        args: &[Expr],
        line: u32,
    ) -> Result<(), String> {
        // Only a name callee `f(args)` is compiled; method/complex callees fall back.
        match &callee.kind {
            ExprKind::Path { segments } if segments.len() == 1 => {
                let name = segments[0].value.clone();
                // Builtins that inspect their arguments unevaluated (spec §19.4) must be rejected
                // before emitting argument code, so no side effect can be replayed on fallback.
                if let Some(b) = Builtin::from_name(&name)
                    && b.needs_unevaluated_args()
                {
                    return Err(format!(
                        "VM compiler: `{name}` requires unevaluated arguments"
                    ));
                }
                for a in args {
                    self.compile_expr(chunk, scope, a)?;
                }
                let idx = chunk.add_name(name)?;
                let argc = checked_u16(args.len(), "call arguments")?;
                chunk.emit(Op::CallName { name: idx, argc }, line);
                Ok(())
            }
            _ => Err("VM compiler: non-name callee unsupported".into()),
        }
    }
}
