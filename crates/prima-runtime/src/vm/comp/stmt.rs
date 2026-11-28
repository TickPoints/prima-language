//! Statement lowering for the bytecode compiler.

use super::*;

impl Compiler {
    pub(super) fn new() -> Compiler {
        Compiler::default()
    }

    /// Compile a `fn` body into a chunk. Parameters are the first slots; the body leaves a value
    /// for `ReturnValue`.
    pub(super) fn compile_fn(&mut self, params: &[Param], body: &Block) -> Result<Chunk, String> {
        let mut chunk = Chunk::new();
        // `self` is an ordinary first parameter in the compiled model; field reads use `Op::Field`
        // and the receiver is passed as the first argument by `call_method_inner` (spec §4.5).
        let arity = checked_u16(params.len(), "parameters")?;
        chunk.slot_count = arity;
        chunk.arity = arity;
        for (i, p) in params.iter().enumerate() {
            chunk.locals.push(Local {
                name: p.name.value.clone(),
                slot: i as u16,
                is_self: p.is_self,
            });
        }
        // Arguments arrive on the operand stack in push order (the chunk's caller pushed them);
        // one `BindParams` distributes them to the parameter slots and removes them.
        if !params.is_empty() {
            chunk.emit(Op::BindParams(arity), 0);
        }
        let mut scope = Scope {
            locals: chunk.locals.clone(),
            slot_count: arity,
            free_temps: Vec::new(),
        };
        let n = body.stmts.len();
        let mut tail_expr = false;
        for (i, stmt) in body.stmts.iter().enumerate() {
            let is_tail_expr = i + 1 == n && matches!(stmt, Stmt::Expr(_));
            self.compile_stmt(&mut chunk, &mut scope, stmt, is_tail_expr)?;
            if is_tail_expr {
                tail_expr = true;
            }
        }
        if chunk.code.is_empty() {
            let n = chunk.add_value(prima_core::Value::Nil)?;
            chunk.emit(Op::Const(n), body.span.start);
            chunk.emit(Op::ReturnValue, body.span.start);
        } else if tail_expr {
            // Implicit return of the trailing expression value left on the stack.
            chunk.emit(Op::ReturnValue, body.span.start);
        } else if !matches!(chunk.code.last(), Some(Op::ReturnValue)) {
            let n = chunk.add_value(prima_core::Value::Nil)?;
            chunk.emit(Op::Const(n), body.span.start);
            chunk.emit(Op::ReturnValue, body.span.start);
        }
        Ok(chunk)
    }

    pub(super) fn compile_stmt(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        stmt: &Stmt,
        is_tail_expr: bool,
    ) -> Result<(), String> {
        match stmt {
            Stmt::Let { pat, value, .. } => {
                // Reserve the slot, compile the initializer into it, then bind the name: a
                // shadowing/self-referential initializer still resolves to the outer binding
                // (the register channel writes the destination directly).
                match pat {
                    Pattern::Binding(name) => {
                        let slot = scope.reserve_slot(chunk)?;
                        self.compile_expr_into(chunk, scope, value, slot)?;
                        scope.define(&name.value, slot);
                        Ok(())
                    }
                    Pattern::Wildcard(_) => {
                        let slot = scope.reserve_slot(chunk)?;
                        self.compile_expr_into(chunk, scope, value, slot)?;
                        scope.define("_", slot);
                        Ok(())
                    }
                    _ => Err("VM compiler: only identifier bindings supported".into()),
                }
            }
            Stmt::Const { name, value, .. } => {
                let slot = scope.reserve_slot(chunk)?;
                self.compile_expr_into(chunk, scope, value, slot)?;
                scope.define(&name.value, slot);
                Ok(())
            }
            Stmt::FnDef { params, body, .. } => {
                // Nested `fn` definitions bind a closure in the environment, not in a slot. The
                // definition itself is delegated to the AST via `EvalStmt`, which keeps the rest of
                // the body compiled. Closures that capture enclosing *locals* (which live only in
                // slots) would see stale/missing bindings, so those stay outside the subset.
                if nested_fn_captures(scope, params, body) {
                    return Err("VM compiler: nested function capture is unsupported".into());
                }
                let id = chunk.add_fallback_stmt(stmt.clone());
                chunk.emit(Op::EvalStmt { stmt: id }, 0);
                Ok(())
            }
            Stmt::Assign {
                target, op, value, ..
            } => self.compile_assign(chunk, scope, target, *op, value),
            Stmt::Expr(e) => {
                // `local.push(arg);` in statement position lowers to a void register push (no
                // operand-stack traffic); the expression form keeps the `MethodLocal` path so it
                // still yields `Nil`.
                if !is_tail_expr && self.try_compile_void_push(chunk, scope, e)? {
                    return Ok(());
                }
                self.compile_expr(chunk, scope, e)?;
                if !is_tail_expr {
                    chunk.emit(Op::Pop, 0);
                }
                Ok(())
            }
            Stmt::Return { value, .. } => {
                match value {
                    Some(e) => self.compile_expr(chunk, scope, e)?,
                    None => {
                        let n = chunk.add_value(prima_core::Value::Nil)?;
                        chunk.emit(Op::Const(n), 0);
                    }
                }
                chunk.emit(Op::ReturnValue, 0);
                Ok(())
            }
            Stmt::If {
                cond,
                then,
                elifs,
                else_,
                ..
            } => self.compile_if(chunk, scope, cond, then, elifs, else_.as_ref()),
            Stmt::While { cond, body, .. } => self.compile_while(chunk, scope, cond, body),
            Stmt::For {
                var,
                range,
                step,
                body,
                ..
            } => self.compile_for(chunk, scope, var, &range.0, &range.1, step.as_ref(), body),
            _ => Err("VM compiler: statement not supported".into()),
        }
    }

    pub(super) fn compile_block(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        block: &Block,
    ) -> Result<(), String> {
        for stmt in &block.stmts {
            self.compile_stmt(chunk, scope, stmt, false)?;
        }
        Ok(())
    }

    pub(super) fn compile_assign(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        target: &Expr,
        op: AssignOp,
        value: &Expr,
    ) -> Result<(), String> {
        match (&target.kind, op) {
            (ExprKind::Path { segments }, AssignOp::Assign) if segments.len() == 1 => {
                let name = &segments[0].value;
                let slot = match scope.slot_of(name) {
                    Some(s) => s,
                    None => scope.alloc(chunk, name)?,
                };
                // Register channel: `x = x + …`, `x = -x`, `x = to_f64(x)`, `x = a[i]` and plain
                // copies avoid the operand stack entirely (spec §12.2).
                self.compile_expr_into(chunk, scope, value, slot)
            }
            (ExprKind::Index { base, index }, AssignOp::Assign) => {
                if index.items.len() != 1 {
                    return Err("VM compiler: multi-dimensional index-assign unsupported".into());
                }
                // The base must be a single-segment path: the executor mutates that binding's
                // array in place, so a lost write on a stack copy is impossible (spec §11.3).
                let name = match &base.kind {
                    ExprKind::Path { segments } if segments.len() == 1 => &segments[0].value,
                    _ => return Err("VM compiler: index-assign target must be a variable".into()),
                };
                let idx = match &index.items[0] {
                    IndexItem::Elem(idx) => idx,
                    IndexItem::Slice { start, end } => {
                        // `arr[lo..hi] = rhs` (spec §11.3): push `lo`/`hi` (a `Nil` sentinel when
                        // omitted) then the right-hand array; the executor splices in place.
                        match start {
                            Some(e) => self.compile_expr(chunk, scope, e)?,
                            None => {
                                let n = chunk.add_value(prima_core::Value::Nil)?;
                                chunk.emit(Op::Const(n), 0);
                            }
                        }
                        match end {
                            Some(e) => self.compile_expr(chunk, scope, e)?,
                            None => {
                                let n = chunk.add_value(prima_core::Value::Nil)?;
                                chunk.emit(Op::Const(n), 0);
                            }
                        }
                        self.compile_expr(chunk, scope, value)?;
                        if let Some(slot) = scope.slot_of(name) {
                            chunk.emit(Op::SliceStoreLocal { slot }, 0);
                        } else {
                            let k = chunk.add_name(name.clone())?;
                            chunk.emit(Op::SliceStoreName { name: k }, 0);
                        }
                        return Ok(());
                    }
                };
                // For an environment binding (non-local target) the executor mutates the binding
                // in place along the chain; this is only faithful when neither the index nor the
                // value expression can run user code that rebinds the target (the AST's
                // conservative whole-value path covers those cases, spec §11.3).
                let is_local = scope.slot_of(name).is_some();
                if !is_local && (!expr_is_side_effect_free(idx) || !expr_is_side_effect_free(value))
                {
                    return Err("VM compiler: index-assign with side effects unsupported".into());
                }
                // Local array + local index: register-form store (no stack traffic, spec §11.3).
                if let Some(arr) = scope.slot_of(name)
                    && let Some(i) = local_slot(scope, idx)
                {
                    if let Some(src) = local_slot(scope, value) {
                        chunk.emit(Op::RegIndexStore { arr, idx: i, src }, 0);
                    } else if let Some(imm) = literal_imm(value) {
                        chunk.emit(Op::RegIndexStoreImm { arr, idx: i, imm }, 0);
                    } else {
                        let t = scope.alloc_temp(chunk)?;
                        self.compile_expr_into(chunk, scope, value, t)?;
                        chunk.emit(
                            Op::RegIndexStore {
                                arr,
                                idx: i,
                                src: t,
                            },
                            0,
                        );
                        scope.free_temp(t);
                    }
                    return Ok(());
                }
                self.compile_expr(chunk, scope, idx)?;
                self.compile_expr(chunk, scope, value)?;
                if let Some(slot) = scope.slot_of(name) {
                    chunk.emit(Op::IndexStoreLocal(slot), 0);
                } else {
                    let k = chunk.add_name(name.clone())?;
                    chunk.emit(Op::IndexStoreName(k), 0);
                }
                chunk.emit(Op::Pop, 0);
                Ok(())
            }
            (ExprKind::Index { base, index }, op @ (AssignOp::AddAssign | AssignOp::SubAssign)) => {
                // `a[i] += v` / `a[i] -= v` (spec §11.3): read-modify-write entirely in the register
                // channel, so no stack copy of the array can be written back by mistake. Slice
                // compound assignment is not meaningful and stays outside the subset.
                if index.items.len() != 1 {
                    return Err("VM compiler: multi-dimensional index-assign unsupported".into());
                }
                let IndexItem::Elem(idx) = &index.items[0] else {
                    return Err("VM compiler: compound slice assignment unsupported".into());
                };
                let name = match &base.kind {
                    ExprKind::Path { segments } if segments.len() == 1 => &segments[0].value,
                    _ => {
                        return Err(
                            "VM compiler: compound index-assign target must be a variable".into(),
                        );
                    }
                };
                let Some(arr) = scope.slot_of(name) else {
                    // Non-local targets use the AST authority (value write-back along the chain).
                    return Err(
                        "VM compiler: compound index-assign to a non-local unsupported".into(),
                    );
                };
                let ai = match op {
                    AssignOp::AddAssign => ArithOp::Add,
                    AssignOp::SubAssign => ArithOp::Sub,
                    _ => unreachable!("matched compound assignment"),
                };
                let idx_slot = scope.alloc_temp(chunk)?;
                if let Err(e) = self.compile_expr_into(chunk, scope, idx, idx_slot) {
                    scope.free_temp(idx_slot);
                    return Err(e);
                }
                let cur = scope.alloc_temp(chunk)?;
                chunk.emit(
                    Op::RegIndex {
                        dst: cur,
                        arr,
                        idx: idx_slot,
                    },
                    0,
                );
                let rhs = scope.alloc_temp(chunk)?;
                if let Err(e) = self.compile_expr_into(chunk, scope, value, rhs) {
                    scope.free_temp(rhs);
                    scope.free_temp(cur);
                    scope.free_temp(idx_slot);
                    return Err(e);
                }
                chunk.emit(
                    Op::RegBin {
                        op: ai,
                        dst: cur,
                        a: cur,
                        b: rhs,
                    },
                    0,
                );
                chunk.emit(
                    Op::RegIndexStore {
                        arr,
                        idx: idx_slot,
                        src: cur,
                    },
                    0,
                );
                scope.free_temp(rhs);
                scope.free_temp(cur);
                scope.free_temp(idx_slot);
                Ok(())
            }
            (ExprKind::Path { segments }, op) if segments.len() == 1 => {
                // Compound `x op= v` → x = x op v (only for a local). Stack order [x, v].
                let name = &segments[0].value;
                let slot = scope
                    .slot_of(name)
                    .ok_or_else(|| "VM compiler: compound assignment to a non-local".to_string())?;
                // `x += <small integer literal>` updates the slot in place with no stack traffic.
                if let (AssignOp::AddAssign, ExprKind::Literal(Literal::Integer(text))) =
                    (op, &value.kind)
                    && let Ok(imm) = text.parse::<i64>()
                {
                    chunk.emit(Op::AddImmLocal { slot, imm }, 0);
                    return Ok(());
                }
                // `x += local` / `x += imm`: a single register-form add (spec §12.2).
                if op == AssignOp::AddAssign {
                    if let Some(b) = local_slot(scope, value) {
                        chunk.emit(
                            Op::RegBin {
                                op: ArithOp::Add,
                                dst: slot,
                                a: slot,
                                b,
                            },
                            0,
                        );
                        return Ok(());
                    }
                    if let Some(imm) = literal_imm(value) {
                        chunk.emit(
                            Op::RegBinImm {
                                op: ArithOp::Add,
                                dst: slot,
                                a: slot,
                                imm,
                            },
                            0,
                        );
                        return Ok(());
                    }
                    // `x += v` is `x = x + v`: the fused `AddToSlot` covers numbers (exact tower),
                    // array concatenation (spec §11.3) and everything else the general path allows.
                    self.compile_expr(chunk, scope, value)?;
                    chunk.emit(Op::AddToSlot(slot), 0);
                    return Ok(());
                }
                chunk.emit(Op::LoadLocal(slot), 0);
                self.compile_expr(chunk, scope, value)?;
                chunk.emit(binary_assign_op(&op)?, 0);
                chunk.emit(Op::SetLocalNc(slot), 0);
                Ok(())
            }
            _ => Err("VM compiler: unsupported assignment target".into()),
        }
    }

    pub(super) fn compile_if(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        cond: &Expr,
        then: &Block,
        elifs: &[(Expr, Block)],
        else_: Option<&Block>,
    ) -> Result<(), String> {
        let mut end_jumps: Vec<usize> = Vec::new();
        let mut cond = cond;
        let mut then_b = then;
        let mut iter = elifs.iter().peekable();
        loop {
            // `if local[i]` fuses the element read and the false branch (spec §11.3/§12.1).
            let else_branch = if let ExprKind::Index { base, index } = &cond.kind
                && index.items.len() == 1
                && let IndexItem::Elem(e) = &index.items[0]
                && let (Some(arr), Some(idx)) = (local_slot(scope, base), local_slot(scope, e))
            {
                CondBranch::IndexFalse(chunk.emit_branch_index_false(arr, idx, cond.span.start))
            } else {
                self.compile_expr(chunk, scope, cond)?;
                CondBranch::JumpIfFalse(chunk.emit_jump_if_false(cond.span.start))
            };
            self.compile_block(chunk, scope, then_b)?;
            // The then branch only needs to jump over what follows; the jump is redundant when
            // this is the last branch and there is no `else`.
            if iter.peek().is_some() || else_.is_some() {
                end_jumps.push(chunk.emit_jump(cond.span.start));
            }
            let else_at = chunk.code.len();
            match else_branch {
                CondBranch::JumpIfFalse(at) => chunk.patch_jump_if_false(at, else_at),
                CondBranch::IndexFalse(at) => chunk.patch_branch_index_false(at, else_at),
            }
            if let Some((c, b)) = iter.next() {
                cond = c;
                then_b = b;
                continue;
            }
            if let Some(e) = else_ {
                self.compile_block(chunk, scope, e)?;
            }
            break;
        }
        let end = chunk.code.len();
        for j in end_jumps {
            chunk.patch_jump(j, end);
        }
        Ok(())
    }

    pub(super) fn compile_while(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        cond: &Expr,
        body: &Block,
    ) -> Result<(), String> {
        let loop_start = chunk.code.len();
        // `while a*b <op> c` with local operands: multiply into a temporary register, then use the
        // fused two-slot branch (e.g. `while i*i <= n`). The operands are re-read every iteration,
        // matching the AST's per-iteration re-evaluation (spec §14).
        if let ExprKind::Binary { op, lhs, rhs } = &cond.kind
            && matches!(op, BinOp::Lt | BinOp::Le)
            && let ExprKind::Binary {
                op: BinOp::Mul,
                lhs: m1,
                rhs: m2,
            } = &lhs.kind
            && let (Some(a), Some(b), Some(c)) = (
                local_slot(scope, m1),
                local_slot(scope, m2),
                local_slot(scope, rhs),
            )
        {
            let t = scope.alloc_temp(chunk)?;
            chunk.emit(
                Op::RegBin {
                    op: ArithOp::Mul,
                    dst: t,
                    a,
                    b,
                },
                cond.span.start,
            );
            let exit = if *op == BinOp::Le {
                chunk.emit_branch_local_le(t, c, cond.span.start)
            } else {
                chunk.emit_branch_local_lt(t, c, cond.span.start)
            };
            self.compile_block(chunk, scope, body)?;
            chunk.emit(Op::Jump(loop_start as i32 - chunk.code.len() as i32), 0);
            if *op == BinOp::Le {
                chunk.patch_branch_local_le(exit, chunk.code.len());
            } else {
                chunk.patch_branch_local_lt(exit, chunk.code.len());
            }
            scope.free_temp(t);
            return Ok(());
        }
        // Fused loop test (spec §14): `while a < b` / `while a <= b` with two local slots compare
        // the slots and jump out in one instruction (the slots are re-read every iteration,
        // matching the AST).
        let fused = local_cmp_operands(scope, cond);
        let exit = match fused {
            Some((true, a, b)) => chunk.emit_branch_local_le(a, b, cond.span.start),
            Some((false, a, b)) => chunk.emit_branch_local_lt(a, b, cond.span.start),
            None => {
                self.compile_expr(chunk, scope, cond)?;
                chunk.emit_jump_if_false(cond.span.start)
            }
        };
        self.compile_block(chunk, scope, body)?;
        chunk.emit(Op::Jump(loop_start as i32 - chunk.code.len() as i32), 0);
        match fused {
            Some((true, ..)) => chunk.patch_branch_local_le(exit, chunk.code.len()),
            Some((false, ..)) => chunk.patch_branch_local_lt(exit, chunk.code.len()),
            None => chunk.patch_jump_if_false(exit, chunk.code.len()),
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn compile_for(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        var: &prima_syntax::ast::Spanned<String>,
        lo: &Expr,
        hi: &Expr,
        step: Option<&Expr>,
        body: &Block,
    ) -> Result<(), String> {
        // Fill idiom: `for _ in lo..hi { local.push(<literal>) }` lowers to one bulk extend. Safe
        // because the body is a single constant push, so the bound cannot change and the loop
        // variable is unused (spec §14).
        if step.is_none()
            && let Some((recv, imm)) = fill_idiom(body)
            && let Some(arr) = scope.slot_of(&recv)
        {
            let (lo_r, lo_t) = self.operand_reg(chunk, scope, lo)?;
            let (hi_r, hi_t) = self.operand_reg(chunk, scope, hi)?;
            chunk.emit(
                Op::RegFill {
                    arr,
                    lo: lo_r,
                    hi: hi_r,
                    imm,
                },
                lo.span.start,
            );
            if hi_t {
                scope.free_temp(hi_r);
            }
            if lo_t {
                scope.free_temp(lo_r);
            }
            return Ok(());
        }
        // `for i in lo..hi` (optionally `step s`) → i = lo; while i < hi { body; i += s(1) }.
        let slot = scope.alloc(chunk, &var.value)?;
        self.compile_expr_into(chunk, scope, lo, slot)?;
        let loop_start = chunk.code.len();
        // Fused loop test when the bound is a plain local slot (re-read every iteration,
        // matching the AST's per-iteration re-evaluation, spec §14). The comparison kind
        // follows the loop: `for i in a..b` tests `i < b` (spec §14).
        let hi_slot = match &hi.kind {
            ExprKind::Path { segments } if segments.len() == 1 => scope.slot_of(&segments[0].value),
            _ => None,
        };
        // `for i in lo..(hi ± k)`: a single fused test against the bound plus/minus an inline
        // constant, without re-materializing the constant each iteration (spec §14).
        let hi_sum = match &hi.kind {
            ExprKind::Binary { op, lhs, rhs } if matches!(op, BinOp::Add | BinOp::Sub) => {
                if let (Some(h), Some(Imm::Int(k))) = (local_slot(scope, lhs), literal_imm(rhs)) {
                    Some((h, if *op == BinOp::Add { k } else { -k }))
                } else {
                    None
                }
            }
            _ => None,
        };
        let exit = match (hi_slot, hi_sum) {
            (Some(h), _) => LoopExit::Lt(chunk.emit_branch_local_lt(slot, h, lo.span.start)),
            (None, Some((h, k))) => {
                LoopExit::CmpSum(chunk.emit_branch_local_cmp_sum(false, slot, h, k, lo.span.start))
            }
            (None, None) => {
                chunk.emit(Op::LoadLocal(slot), 0);
                self.compile_expr(chunk, scope, hi)?;
                chunk.emit(Op::Lt, 0);
                LoopExit::Jump(chunk.emit_jump_if_false(lo.span.start))
            }
        };
        self.compile_block(chunk, scope, body)?;
        // Loop increment through the register channel: the default `step 1` is a single in-place
        // add, and a local/literal step is one register op (spec §14).
        match step {
            None => chunk.emit(Op::AddImmLocal { slot, imm: 1 }, 0),
            Some(s) => {
                if let Some(b) = local_slot(scope, s) {
                    chunk.emit(
                        Op::RegBin {
                            op: ArithOp::Add,
                            dst: slot,
                            a: slot,
                            b,
                        },
                        0,
                    );
                } else if let Some(Imm::Int(v)) = literal_imm(s) {
                    chunk.emit(Op::AddImmLocal { slot, imm: v }, 0);
                } else if let Some(imm) = literal_imm(s) {
                    chunk.emit(
                        Op::RegBinImm {
                            op: ArithOp::Add,
                            dst: slot,
                            a: slot,
                            imm,
                        },
                        0,
                    );
                } else {
                    self.compile_expr(chunk, scope, s)?;
                    chunk.emit(Op::AddToSlot(slot), 0);
                }
            }
        }
        chunk.emit(Op::Jump(loop_start as i32 - chunk.code.len() as i32), 0);
        match exit {
            LoopExit::Lt(at) => chunk.patch_branch_local_lt(at, chunk.code.len()),
            LoopExit::CmpSum(at) => chunk.patch_branch_local_cmp_sum(at, chunk.code.len()),
            LoopExit::Jump(at) => chunk.patch_jump_if_false(at, chunk.code.len()),
        }
        Ok(())
    }

    /// If `e` is `local.push(arg)` (spec §11.3), emit a void register-form push and return `true`
    /// so the caller can skip the statement's trailing `Pop`. Only valid in statement position —
    /// the expression form stays on the stack `MethodLocal` path so it still yields `Nil`.
    pub(super) fn try_compile_void_push(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        e: &Expr,
    ) -> Result<bool, String> {
        let ExprKind::MethodCall {
            receiver,
            name,
            args,
        } = &e.kind
        else {
            return Ok(false);
        };
        if name.value != "push" || args.len() != 1 {
            return Ok(false);
        }
        let ExprKind::Path { segments } = &receiver.kind else {
            return Ok(false);
        };
        if segments.len() != 1 {
            return Ok(false);
        }
        let Some(arr) = scope.slot_of(&segments[0].value) else {
            return Ok(false);
        };
        if let Some(src) = local_slot(scope, &args[0]) {
            chunk.emit(Op::RegPush { arr, src }, e.span.start);
        } else if let Some(imm) = literal_imm(&args[0]) {
            chunk.emit(Op::RegPushImm { arr, imm }, e.span.start);
        } else {
            let t = scope.alloc_temp(chunk)?;
            self.compile_expr_into(chunk, scope, &args[0], t)?;
            chunk.emit(Op::RegPush { arr, src: t }, e.span.start);
            scope.free_temp(t);
        }
        Ok(true)
    }
}
