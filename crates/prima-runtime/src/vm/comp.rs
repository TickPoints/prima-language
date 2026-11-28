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
    AssignOp, BinOp, Block, ClassMemberKind, Expr, ExprKind, IndexItem, Literal, Param, Pattern,
    Program, Stmt, UnOp,
};

use super::op::{ArithOp, Chunk, Imm, Local, Op, Program as VmProgram};
use crate::builtins::Builtin;
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
        Literal::String { value, .. } => Some(prima_core::Value::String(value.clone().into())),
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

/// Whether a nested `fn` body reads a name bound to an enclosing VM local slot (which lives only in
/// the frame, not in the environment). Conservative: any statement shape the scan does not model is
/// treated as a capture, so the caller falls back to the AST instead of changing semantics.
fn nested_fn_captures(scope: &Scope, params: &[Param], body: &Block) -> bool {
    let mut names = std::collections::HashSet::new();
    if scan_block(body, &mut names) {
        return true;
    }
    names
        .into_iter()
        .any(|name| !params.iter().any(|p| p.name.value == name) && scope.slot_of(&name).is_some())
}

/// Collect read names from a block; returns `true` when an unsupported shape was encountered.
fn scan_block(block: &Block, out: &mut std::collections::HashSet<String>) -> bool {
    block.stmts.iter().any(|s| scan_stmt(s, out))
}

fn scan_stmt(stmt: &Stmt, out: &mut std::collections::HashSet<String>) -> bool {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Const { value, .. } => {
            crate::eval::collect_read_names(value, out);
            false
        }
        Stmt::FnDef { body, .. } => scan_block(body, out),
        Stmt::MathDef { body, .. } => {
            crate::eval::collect_read_names(body, out);
            false
        }
        Stmt::ClassDef { members, .. } => {
            let mut unknown = false;
            for m in members {
                if let ClassMemberKind::Method {
                    body: Some(body), ..
                } = &m.kind
                {
                    unknown |= scan_block(body, out);
                }
            }
            unknown
        }
        Stmt::Impl { members, .. } => members.iter().any(|m| scan_stmt(m, out)),
        Stmt::Assign { target, value, .. } => {
            crate::eval::collect_read_names(target, out);
            crate::eval::collect_read_names(value, out);
            false
        }
        Stmt::Expr(e) => {
            crate::eval::collect_read_names(e, out);
            false
        }
        Stmt::For {
            range, step, body, ..
        }
        | Stmt::ParFor {
            range, step, body, ..
        } => {
            crate::eval::collect_read_names(&range.0, out);
            crate::eval::collect_read_names(&range.1, out);
            if let Some(step) = step {
                crate::eval::collect_read_names(step, out);
            }
            scan_block(body, out)
        }
        Stmt::While { cond, body, .. } => {
            crate::eval::collect_read_names(cond, out);
            scan_block(body, out)
        }
        Stmt::If {
            cond,
            then,
            elifs,
            else_,
            ..
        } => {
            crate::eval::collect_read_names(cond, out);
            if scan_block(then, out) {
                return true;
            }
            for (cond, block) in elifs {
                crate::eval::collect_read_names(cond, out);
                if scan_block(block, out) {
                    return true;
                }
            }
            else_.as_ref().is_some_and(|b| scan_block(b, out))
        }
        Stmt::IfLet {
            value, then, else_, ..
        } => {
            crate::eval::collect_read_names(value, out);
            scan_block(then, out) || else_.as_ref().is_some_and(|b| scan_block(b, out))
        }
        Stmt::WhileLet { value, body, .. } => {
            crate::eval::collect_read_names(value, out);
            scan_block(body, out)
        }
        Stmt::Match {
            scrutinee, arms, ..
        } => {
            crate::eval::collect_read_names(scrutinee, out);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    crate::eval::collect_read_names(guard, out);
                }
                crate::eval::collect_read_names(&arm.body, out);
            }
            false
        }
        Stmt::Return { value, .. } => {
            if let Some(value) = value {
                crate::eval::collect_read_names(value, out);
            }
            false
        }
        Stmt::WithConfig { body, .. } => scan_block(body, out),
        Stmt::Pub(inner) => scan_stmt(inner, out),
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
    fn unevaluated_argument_builtins_reject_compilation() {
        for name in [
            "derivative",
            "partial",
            "grad",
            "limit",
            "jit",
            "map",
            "filter",
            "reduce",
        ] {
            let src = format!("fn f() -> Integer {{ {name}(); 0 }}");
            let prog = prima_syntax::parse(&src).expect("parse");
            let Stmt::FnDef { params, body, .. } = &prog.stmts[0] else {
                panic!("expected a top-level fn");
            };
            assert!(
                compile_function_body(params, body).is_err(),
                "`{name}` must be rejected before its arguments are evaluated"
            );
        }
    }

    #[test]
    fn self_parameter_and_field_read_compile() {
        let chunk = fn_body("pub fn get(self) -> Integer { self.value }");
        assert!(chunk.code.iter().any(|op| matches!(op, Op::Field { .. })));
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

mod expr;
mod stmt;
