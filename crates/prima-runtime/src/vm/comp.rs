//! AST → bytecode compiler (spec §19.5, Milestone B).
//!
//! `compile_program` lowers a parsed `Program` into a `Program` of chunks: a root chunk (top-level
//! statements) plus one chunk per top-level `fn`. Statements/expressions are lowered bottom-up onto
//! the VM operand stack; locals are slot-allocated. This milestone supports a conservative but real
//! subset (numeric/string/bool literals, local loads, global/builtin name loads, binary/unary ops,
//! `let`, assignment, `if`/`while`/`for`/`return`, array/tuple literals, index read, and calls by
//! name / method calls). Any construct outside the subset causes the whole program to be rejected so
//! the caller falls back to the AST interpreter — observable behavior is never changed.
//!
//! All value-producing commands delegate their semantics to the `Evaluator` at runtime (the executor
//! calls `eval_binary`/`eval_compare`/`call_method`/`apply_function`), so VM results equal AST
//! results by construction.

use std::rc::Rc;

use rustc_hash::FxHashMap;

use prima_syntax::ast::{
    AssignOp, BinOp, Block, Expr, ExprKind, IndexItem, Literal, Param, Pattern, Program, Stmt, UnOp,
};

use super::op::{ArithOp, Chunk, Imm, Local, Op, Program as VmProgram};
use crate::eval::expr_is_side_effect_free;

/// Checked `usize` → `u16` conversion for VM operand fields (`Reg`/`argc`/element counts). A count
/// beyond the instruction set's range rejects compilation, so the caller falls back to the AST
/// interpreter rather than silently truncating (spec §19.5).
fn checked_u16(n: usize, what: &str) -> Result<u16, String> {
    u16::try_from(n).map_err(|_| format!("VM compiler: too many {what} for the u16 limit"))
}

/// Compile a single `fn` body (already-parametrized) into a chunk, with an empty local scope beyond
/// the parameters. Used by the evaluator's per-function VM fast path behind `vm := true`.
pub fn compile_function_body(params: &[Param], body: &Block) -> Result<Chunk, String> {
    let mut comp = Compiler::new();
    comp.compile_fn(params, body)
}

/// Compile a `Program` into a VM program. Unsupported constructs are rejected with `Err(String)`,
/// in which case the caller falls back to the AST interpreter for the whole program.
pub fn compile_program(ast: &Program) -> Result<VmProgram, String> {
    let mut comp = Compiler::new();
    for stmt in &ast.stmts {
        if let Stmt::FnDef {
            name, body, params, ..
        } = stmt
        {
            let chunk = comp.compile_fn(params, body)?;
            comp.functions.push(chunk);
            let idx = (comp.functions.len() - 1) as u32;
            comp.names.insert(name.value.clone(), idx);
        }
    }
    // Root chunk: top-level non-`fn` statements.
    let mut root = Chunk::new();
    let mut scope = Scope::root();
    for stmt in &ast.stmts {
        if matches!(stmt, Stmt::FnDef { .. }) {
            continue;
        }
        comp.compile_stmt(&mut root, &mut scope, stmt, false)?;
    }
    if root.code.is_empty() {
        let n = root.add_value(prima_core::Value::Nil)?;
        root.emit(Op::Const(n), 0);
    }
    root.lines.push(0);
    Ok(VmProgram {
        root: Rc::new(root),
        functions: comp.functions.into_iter().map(Rc::new).collect(),
        names: comp.names,
    })
}

/// Compiler state: function chunks built so far and their name table.
#[derive(Default)]
struct Compiler {
    functions: Vec<Chunk>,
    names: FxHashMap<String, u32>,
}

impl Compiler {
    fn new() -> Compiler {
        Compiler::default()
    }

    /// Compile a `fn` body into a chunk. Parameters are the first slots; the body leaves a value
    /// for `ReturnValue`.
    fn compile_fn(&mut self, params: &[Param], body: &Block) -> Result<Chunk, String> {
        let mut chunk = Chunk::new();
        if params.iter().any(|p| p.is_self) {
            // `self` receivers are class methods, which the compiled subset does not lower here.
            return Err("VM compiler: `self` parameters unsupported".into());
        }
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

    fn compile_stmt(
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
            Stmt::FnDef { .. } => {
                // Nested `fn` definitions are bound through the environment at runtime, which the
                // compiled subset does not model (locals live in slots) — reject so the whole
                // program falls back to the AST interpreter.
                Err("VM compiler: nested function definitions unsupported".into())
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

    fn compile_block(
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

    fn compile_assign(
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
                let IndexItem::Elem(idx) = &index.items[0] else {
                    return Err("VM compiler: slice index-assign unsupported".into());
                };
                // The base must be a single-segment path: the executor mutates that binding's
                // array in place, so a lost write on a stack copy is impossible (spec §11.3).
                let name = match &base.kind {
                    ExprKind::Path { segments } if segments.len() == 1 => &segments[0].value,
                    _ => return Err("VM compiler: index-assign target must be a variable".into()),
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

    fn compile_if(
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

    fn compile_while(
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
    fn compile_for(
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
    fn try_compile_void_push(
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

    /// Compile `expr` directly into local slot `dst` (the register channel, spec §19.5). Shapes
    /// with a register form — arithmetic over locals/immediates, `to_f64`, negation, indexing,
    /// plain moves — emit register instructions with no operand-stack traffic; everything else
    /// falls back to the stack form plus `SetLocalNc`. `dst` may alias an operand slot (the
    /// executor copies operands before writing).
    fn compile_expr_into(
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
    fn operand_reg(
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

    fn compile_expr(
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
                if crate::eval::is_mutating_array_method(&name.value) {
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
                            chunk.emit(Op::MethodName { name: idx, argc }, expr.span.start);
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
            ExprKind::Index { base, index } => {
                if index.items.len() != 1 {
                    return Err("VM compiler: multi-dimensional indexing unsupported".into());
                }
                // Condition/expression reads with a local base and index push the element directly
                // (no receiver-handle clone): `if prime[i]` (spec §11.3).
                if let IndexItem::Elem(e) = &index.items[0]
                    && let (Some(arr), Some(idx)) = (local_slot(scope, base), local_slot(scope, e))
                {
                    chunk.emit(Op::RegIndexPush { arr, idx }, expr.span.start);
                    return Ok(());
                }
                self.compile_expr(chunk, scope, base)?;
                match &index.items[0] {
                    IndexItem::Elem(e) => {
                        self.compile_expr(chunk, scope, e)?;
                        chunk.emit(Op::Index, expr.span.start);
                    }
                    IndexItem::Slice { .. } => {
                        return Err("VM compiler: slice indexing unsupported".into());
                    }
                }
                Ok(())
            }
            _ => Err("VM compiler: expression unsupported".into()),
        }
    }

    fn compile_call(
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

/// Compile-time scope: live local slots plus the running slot count (merged into the chunk).
struct Scope {
    locals: Vec<Local>,
    slot_count: u16,
    /// Temporaries freed by `free_temp` and reused by `alloc_temp` (keeps the slot count bounded).
    free_temps: Vec<u16>,
}

impl Scope {
    fn root() -> Scope {
        Scope {
            locals: Vec::new(),
            slot_count: 0,
            free_temps: Vec::new(),
        }
    }

    fn slot_of(&self, name: &str) -> Option<u16> {
        self.locals
            .iter()
            .rev()
            .find(|l| l.name == name)
            .map(|l| l.slot)
    }

    /// Reserve a slot without binding a name: the caller compiles the initializer first and then
    /// calls [`Self::define`], so a self-referential `let x = x + 1` still resolves `x` to the
    /// outer/undefined binding (matching the AST interpreter).
    fn reserve_slot(&mut self, chunk: &mut Chunk) -> Result<u16, String> {
        let slot = self.slot_count;
        let next = slot
            .checked_add(1)
            .ok_or_else(|| "VM compiler: too many local slots".to_string())?;
        self.slot_count = next;
        if chunk.slot_count <= slot {
            chunk.slot_count = next;
        }
        chunk.locals.push(Local {
            name: "__slot".to_string(),
            slot,
            is_self: false,
        });
        Ok(slot)
    }

    /// Bind `name` to an already-reserved slot.
    fn define(&mut self, name: &str, slot: u16) {
        self.locals.push(Local {
            name: name.to_string(),
            slot,
            is_self: false,
        });
    }

    fn alloc(&mut self, chunk: &mut Chunk, name: &str) -> Result<u16, String> {
        let slot = self.reserve_slot(chunk)?;
        self.define(name, slot);
        Ok(slot)
    }

    /// Allocate a reusable temporary slot (register-form compilation).
    fn alloc_temp(&mut self, chunk: &mut Chunk) -> Result<u16, String> {
        if let Some(t) = self.free_temps.pop() {
            return Ok(t);
        }
        self.reserve_slot(chunk)
    }

    /// Return a temporary slot to the reuse pool.
    fn free_temp(&mut self, slot: u16) {
        self.free_temps.push(slot);
    }
}

fn is_bool_literal(name: &str) -> bool {
    name == "true" || name == "false"
}

fn compile_literal(chunk: &mut Chunk, lit: &Literal) -> Result<u16, String> {
    let v = literal_to_value(lit).ok_or_else(|| "VM compiler: unsupported literal".to_string())?;
    chunk.add_value(v)
}

fn literal_to_value(lit: &Literal) -> Option<prima_core::Value> {
    use prima_core::{Number, Real};
    match lit {
        // Integer literals narrow to the inlined `Small` representation when they fit `i64`
        // (spec §6.1), so hot loops never widen through the heap-allocated `Integer` layer.
        Literal::Integer(s) => Some(prima_core::Value::Number(match s.parse::<i64>() {
            Ok(v) => Number::Small(v),
            Err(_) => Number::from_bigint(s.parse().ok()?),
        })),
        Literal::Float(s) => Some(prima_core::Value::Number(Number::Real(Real::F64(
            s.parse().ok()?,
        )))),
        Literal::Bool(b) => Some(prima_core::Value::Bool(*b)),
        Literal::String { value, .. } => Some(prima_core::Value::String(value.clone())),
        Literal::Char(c) => Some(prima_core::Value::Char(*c)),
        _ => None,
    }
}

/// A loop-exit branch emitted by `compile_for`, recorded so its placeholder can be patched.
enum LoopExit {
    Lt(usize),
    CmpSum(usize),
    Jump(usize),
}

/// A conditional branch emitted by `compile_if`, recorded so its placeholder can be patched.
enum CondBranch {
    JumpIfFalse(usize),
    IndexFalse(usize),
}

/// Detect the fill idiom `for _ in lo..hi { local.push(<literal>) }`: the body must be exactly one
/// statement (a constant push), so lowering it to a bulk extend cannot change behavior.
fn fill_idiom(body: &Block) -> Option<(String, Imm)> {
    if body.stmts.len() != 1 {
        return None;
    }
    let Stmt::Expr(e) = &body.stmts[0] else {
        return None;
    };
    let ExprKind::MethodCall {
        receiver,
        name,
        args,
    } = &e.kind
    else {
        return None;
    };
    if name.value != "push" || args.len() != 1 {
        return None;
    }
    let ExprKind::Path { segments } = &receiver.kind else {
        return None;
    };
    if segments.len() != 1 {
        return None;
    }
    let imm = literal_imm(&args[0])?;
    Some((segments[0].value.clone(), imm))
}

/// The local slot a single-segment path expression refers to, if any (register channel).
fn local_slot(scope: &Scope, e: &Expr) -> Option<u16> {
    match &e.kind {
        ExprKind::Path { segments } if segments.len() == 1 => scope.slot_of(&segments[0].value),
        _ => None,
    }
}

/// The inline immediate an integer/float/bool literal expression denotes, if any. `true`/`false`
/// reach the parser as single-segment paths (the AST's bool-literal form).
fn literal_imm(e: &Expr) -> Option<Imm> {
    match &e.kind {
        ExprKind::Literal(Literal::Integer(s)) => s.parse::<i64>().ok().map(Imm::Int),
        ExprKind::Literal(Literal::Float(s)) => s.parse::<f64>().ok().map(Imm::float),
        ExprKind::Literal(Literal::Bool(b)) => Some(Imm::Bool(*b)),
        ExprKind::Path { segments } if segments.len() == 1 => match segments[0].value.as_str() {
            "true" => Some(Imm::Bool(true)),
            "false" => Some(Imm::Bool(false)),
            _ => None,
        },
        _ => None,
    }
}

/// The register arithmetic op for a binary operator, or `None` for non-arithmetic operators.
fn arith_op(op: &BinOp) -> Option<ArithOp> {
    match op {
        BinOp::Add => Some(ArithOp::Add),
        BinOp::Sub => Some(ArithOp::Sub),
        BinOp::Mul => Some(ArithOp::Mul),
        BinOp::Div => Some(ArithOp::Div),
        BinOp::Mod => Some(ArithOp::Rem),
        _ => None,
    }
}

/// If `cond` is `a < b` or `a <= b` with both operands single-segment local slots, return the
/// kind (true = `<=`) and their slots — the fused `BranchLocal{Lt,Le}` forms (spec §14).
/// Non-local paths stay on the general path.
fn local_cmp_operands(scope: &Scope, cond: &Expr) -> Option<(bool, u16, u16)> {
    let (le, lhs, rhs) = match &cond.kind {
        ExprKind::Binary {
            op: BinOp::Lt,
            lhs,
            rhs,
        } => (false, lhs, rhs),
        ExprKind::Binary {
            op: BinOp::Le,
            lhs,
            rhs,
        } => (true, lhs, rhs),
        _ => return None,
    };
    let slot_of = |e: &Expr| match &e.kind {
        ExprKind::Path { segments } if segments.len() == 1 => scope.slot_of(&segments[0].value),
        _ => None,
    };
    match (slot_of(lhs), slot_of(rhs)) {
        (Some(a), Some(b)) => Some((le, a, b)),
        _ => None,
    }
}

/// The `Op` for a compound assignment operator (`x op= v`).
fn binary_assign_op(op: &AssignOp) -> Result<Op, String> {
    use AssignOp::*;
    Ok(match op {
        AddAssign => Op::Add,
        SubAssign => Op::Sub,
        Assign => return Err("VM compiler: `=` is not a compound operator".into()),
    })
}

/// The `Op` emitted for a supported binary operator. `And`/`Or` are handled by the executor with
/// short-circuit semantics; `In`/set-algebra/`MatMul`/`Broadcast` are not supported here.
fn binary_op(op: &BinOp) -> Result<Op, String> {
    use BinOp::*;
    Ok(match op {
        Add => Op::Add,
        Sub => Op::Sub,
        Mul => Op::Mul,
        Div => Op::Div,
        Pow => Op::Pow,
        Mod => Op::Rem,
        Eq => Op::EqCmp,
        Ne => Op::NeCmp,
        Lt => Op::Lt,
        Le => Op::Le,
        Gt => Op::Gt,
        Ge => Op::Ge,
        And => Op::And,
        Or => Op::Or,
        In | Union | Intersect | Difference | MatMul | Broadcast => {
            return Err("VM compiler: unsupported binary operator".into());
        }
    })
}

fn unary_op(op: UnOp) -> Op {
    match op {
        UnOp::Neg => Op::Neg,
        UnOp::Not => Op::Not,
        UnOp::Pos => Op::Dup,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile the body of the first (possibly `pub`) top-level `fn`.
    fn fn_body(src: &str) -> Chunk {
        let prog = prima_syntax::parse(src).expect("parse");
        for stmt in &prog.stmts {
            let inner = match stmt {
                Stmt::Pub(i) => i.as_ref(),
                other => other,
            };
            if let Stmt::FnDef { params, body, .. } = inner {
                return compile_function_body(params, body).expect("compile");
            }
        }
        panic!("no top-level fn");
    }

    #[test]
    fn register_channel_used_for_numeric_loop() {
        let chunk = fn_body(
            "pub fn main(n: Integer) -> Integer { let mut s = 0; let mut i = 0; while i < n { s = s + i * i; i += 1; } s }",
        );
        assert!(
            chunk
                .code
                .iter()
                .any(|op| matches!(op, Op::RegMulAdd { .. }))
        );
        assert!(
            chunk
                .code
                .iter()
                .any(|op| matches!(op, Op::BranchLocalLt { .. }))
        );
        assert!(!chunk.code.iter().any(|op| matches!(op, Op::Add)));
    }

    #[test]
    fn fill_idiom_and_index_branch_are_emitted() {
        let chunk = fn_body(
            "pub fn main(n: Integer) -> Integer { let mut a = []; for k in 0..(n+1) { a.push(true); } let mut c = 0; for k in 0..(n+1) { if a[k] { c += 1; } } c }",
        );
        assert!(chunk.code.iter().any(|op| matches!(op, Op::RegFill { .. })));
        assert!(
            chunk
                .code
                .iter()
                .any(|op| matches!(op, Op::RegIndexBranchFalse { .. }))
        );
    }

    #[test]
    fn to_f64_and_push_lower_to_register_ops() {
        let chunk = fn_body(
            "pub fn main(n: Integer) -> F64 { let mut x = []; for i in 0..n { x.push(to_f64(i)); } let mut s = 0.0; for i in 0..n { s = s + x[i]; } s }",
        );
        assert!(
            chunk
                .code
                .iter()
                .any(|op| matches!(op, Op::RegToF64 { .. }))
        );
        assert!(chunk.code.iter().any(|op| matches!(op, Op::RegPush { .. })));
        assert!(
            chunk
                .code
                .iter()
                .any(|op| matches!(op, Op::RegIndex { .. }))
        );
    }
}
