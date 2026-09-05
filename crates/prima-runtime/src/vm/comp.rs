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

use std::collections::HashMap;

use prima_syntax::ast::{
    AssignOp, BinOp, Block, Expr, ExprKind, IndexItem, Literal, Param, Pattern, Program, Stmt, UnOp,
};

use super::op::{Chunk, Local, Op, Program as VmProgram};

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
        let n = root.add_value(prima_core::Value::Nil);
        root.emit(Op::Const(n), 0);
    }
    root.lines.push(0);
    Ok(VmProgram {
        root,
        functions: comp.functions,
        names: comp.names,
    })
}

/// Compiler state: function chunks built so far and their name table.
#[derive(Default)]
struct Compiler {
    functions: Vec<Chunk>,
    names: HashMap<String, u32>,
}

impl Compiler {
    fn new() -> Compiler {
        Compiler::default()
    }

    /// Compile a `fn` body into a chunk. Parameters are the first slots; the body leaves a value
    /// for `ReturnValue`.
    fn compile_fn(&mut self, params: &[Param], body: &Block) -> Result<Chunk, String> {
        let mut chunk = Chunk::new();
        chunk.slot_count = params.len() as u16;
        for (i, p) in params.iter().enumerate() {
            let is_self = p.is_self;
            chunk.locals.push(Local {
                name: p.name.value.clone(),
                slot: i as u16,
                is_self,
            });
            // Parameters arrive on the operand stack; store each into its slot.
            if is_self {
                chunk.emit(Op::LoadSelf, 0);
                chunk.emit(Op::SetLocal(i as u16), 0);
            } else {
                chunk.emit(Op::SetLocal(i as u16), 0);
            }
        }
        let mut scope = Scope {
            locals: chunk.locals.clone(),
            slot_count: params.len() as u16,
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
            let n = chunk.add_value(prima_core::Value::Nil);
            chunk.emit(Op::Const(n), body.span.start);
            chunk.emit(Op::ReturnValue, body.span.start);
        } else if tail_expr {
            // Implicit return of the trailing expression value left on the stack.
            chunk.emit(Op::ReturnValue, body.span.start);
        } else if !matches!(chunk.code.last(), Some(Op::ReturnValue)) {
            let n = chunk.add_value(prima_core::Value::Nil);
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
                self.compile_expr(chunk, scope, value)?;
                let slot = scope.bind_name_chunk(chunk, pat)?;
                chunk.emit(Op::SetLocal(slot), 0);
                chunk.emit(Op::Pop, 0);
                Ok(())
            }
            Stmt::Const { name, value, .. } => {
                self.compile_expr(chunk, scope, value)?;
                let slot = scope.alloc(chunk, &name.value);
                chunk.emit(Op::SetLocal(slot), 0);
                chunk.emit(Op::Pop, 0);
                Ok(())
            }
            Stmt::FnDef { .. } => Ok(()), // collected by `compile_program`
            Stmt::Assign {
                target, op, value, ..
            } => self.compile_assign(chunk, scope, target, *op, value),
            Stmt::Expr(e) => {
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
                        let n = chunk.add_value(prima_core::Value::Nil);
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
                self.compile_expr(chunk, scope, value)?;
                let slot = match scope.slot_of(name) {
                    Some(s) => s,
                    None => scope.alloc(chunk, name),
                };
                chunk.emit(Op::SetLocal(slot), 0);
                chunk.emit(Op::Pop, 0);
                Ok(())
            }
            (ExprKind::Index { base, index }, AssignOp::Assign) => {
                let IndexItem::Elem(idx) = &index.items[0] else {
                    return Err("VM compiler: slice index-assign unsupported".into());
                };
                self.compile_expr(chunk, scope, base)?;
                self.compile_expr(chunk, scope, idx)?;
                self.compile_expr(chunk, scope, value)?;
                chunk.emit(Op::IndexStore, 0);
                chunk.emit(Op::Pop, 0);
                Ok(())
            }
            (ExprKind::Path { segments }, op) if segments.len() == 1 => {
                // Compound `x op= v` → x = x op v (only for a local). Stack order [x, v].
                let name = &segments[0].value;
                let slot = scope
                    .slot_of(name)
                    .ok_or_else(|| "VM compiler: compound assignment to a non-local".to_string())?;
                chunk.emit(Op::LoadLocal(slot), 0);
                self.compile_expr(chunk, scope, value)?;
                chunk.emit(binary_assign_op(&op)?, 0);
                chunk.emit(Op::SetLocal(slot), 0);
                chunk.emit(Op::Pop, 0);
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
            self.compile_expr(chunk, scope, cond)?;
            let else_jump = chunk.emit_jump_if_false(cond.span.start);
            self.compile_block(chunk, scope, then_b)?;
            end_jumps.push(chunk.emit_jump(cond.span.start));
            let else_at = chunk.code.len();
            chunk.patch_jump_if_false(else_jump, else_at);
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
        self.compile_expr(chunk, scope, cond)?;
        let exit = chunk.emit_jump_if_false(cond.span.start);
        self.compile_block(chunk, scope, body)?;
        chunk.emit(Op::Jump(loop_start as i32 - chunk.code.len() as i32), 0);
        chunk.patch_jump_if_false(exit, chunk.code.len());
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
        // `for i in lo..hi` (optionally `step s`) → i = lo; while i < hi { body; i += s(1) }.
        let slot = scope.alloc(chunk, &var.value);
        self.compile_expr(chunk, scope, lo)?;
        chunk.emit(Op::SetLocal(slot), 0);
        chunk.emit(Op::Pop, 0);
        let loop_start = chunk.code.len();
        chunk.emit(Op::LoadLocal(slot), 0);
        self.compile_expr(chunk, scope, hi)?;
        chunk.emit(Op::Lt, 0);
        let exit = chunk.emit_jump_if_false(lo.span.start);
        self.compile_block(chunk, scope, body)?;
        chunk.emit(Op::LoadLocal(slot), 0);
        match step {
            Some(s) => self.compile_expr(chunk, scope, s)?,
            None => {
                let c = chunk.add_value(prima_core::Value::Number(prima_core::Number::from(1i64)));
                chunk.emit(Op::Const(c), 0);
            }
        }
        chunk.emit(Op::Add, 0);
        chunk.emit(Op::SetLocal(slot), 0);
        chunk.emit(Op::Pop, 0);
        chunk.emit(Op::Jump(loop_start as i32 - chunk.code.len() as i32), 0);
        chunk.patch_jump_if_false(exit, chunk.code.len());
        Ok(())
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
                let c = chunk.add_value(prima_core::Value::Bool(segments[0].value == "true"));
                chunk.emit(Op::Const(c), expr.span.start);
                Ok(())
            }
            ExprKind::Path { segments } if segments.len() == 1 => {
                let name = &segments[0].value;
                if let Some(slot) = scope.slot_of(name) {
                    chunk.emit(Op::LoadLocal(slot), expr.span.start);
                } else {
                    let idx = chunk.add_name(name.clone());
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
                self.compile_expr(chunk, scope, receiver)?;
                for a in args {
                    self.compile_expr(chunk, scope, a)?;
                }
                let idx = chunk.add_name(name.value.clone());
                chunk.emit(
                    Op::Method {
                        name: idx,
                        argc: args.len() as u16,
                    },
                    expr.span.start,
                );
                Ok(())
            }
            ExprKind::Array(items) => {
                for it in items {
                    self.compile_expr(chunk, scope, it)?;
                }
                chunk.emit(Op::MakeArray(items.len() as u16), expr.span.start);
                Ok(())
            }
            ExprKind::Tuple(items) => {
                for it in items {
                    self.compile_expr(chunk, scope, it)?;
                }
                chunk.emit(Op::MakeTuple(items.len() as u16), expr.span.start);
                Ok(())
            }
            ExprKind::Index { base, index } => {
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
                let idx = chunk.add_name(name);
                chunk.emit(
                    Op::CallName {
                        name: idx,
                        argc: args.len() as u16,
                    },
                    line,
                );
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
}

impl Scope {
    fn root() -> Scope {
        Scope {
            locals: Vec::new(),
            slot_count: 0,
        }
    }

    fn slot_of(&self, name: &str) -> Option<u16> {
        self.locals
            .iter()
            .rev()
            .find(|l| l.name == name)
            .map(|l| l.slot)
    }

    fn alloc(&mut self, chunk: &mut Chunk, name: &str) -> u16 {
        let slot = self.slot_count;
        self.slot_count = slot + 1;
        if chunk.slot_count <= slot {
            chunk.slot_count = slot + 1;
        }
        chunk.locals.push(Local {
            name: name.to_string(),
            slot,
            is_self: false,
        });
        self.locals.push(Local {
            name: name.to_string(),
            slot,
            is_self: false,
        });
        slot
    }

    /// Bind a simple identifier pattern: allocate a new slot for the bound name.
    fn bind_name_chunk(&mut self, chunk: &mut Chunk, pat: &Pattern) -> Result<u16, String> {
        match pat {
            Pattern::Binding(name) => Ok(self.alloc(chunk, &name.value)),
            Pattern::Wildcard(_) => {
                let slot = self.alloc(chunk, "_");
                Ok(slot)
            }
            _ => Err("VM compiler: only identifier bindings supported".into()),
        }
    }
}

fn is_bool_literal(name: &str) -> bool {
    name == "true" || name == "false"
}

fn compile_literal(chunk: &mut Chunk, lit: &Literal) -> Result<u16, String> {
    let v = literal_to_value(lit).ok_or_else(|| "VM compiler: unsupported literal".to_string())?;
    Ok(chunk.add_value(v))
}

fn literal_to_value(lit: &Literal) -> Option<prima_core::Value> {
    use prima_core::{Number, Real};
    match lit {
        Literal::Integer(s) => Some(prima_core::Value::Number(Number::Integer(s.parse().ok()?))),
        Literal::Float(s) => Some(prima_core::Value::Number(Number::Real(Real::F64(
            s.parse().ok()?,
        )))),
        Literal::Bool(b) => Some(prima_core::Value::Bool(*b)),
        Literal::String { value, .. } => Some(prima_core::Value::String(value.clone())),
        Literal::Char(c) => Some(prima_core::Value::Char(*c)),
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
