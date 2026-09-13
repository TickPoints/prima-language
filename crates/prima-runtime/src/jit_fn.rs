//! Whole-function JIT lowering (spec §19.2): translate a pure numeric `fn` body into the typed
//! register IR consumed by `prima_jit::func::compile_ir`.
//!
//! Only the numeric subset is lowered: `I64`/`F64`/`Bool` locals and dense local arrays of those
//! element types, arithmetic/comparisons, `to_f64`, `if`/`while`/`for`/`return`, `local.push`,
//! `local.len()` and indexing. Anything else makes the lowering return `None`, and the caller
//! stays on the interpreter.

use std::collections::HashMap;

use prima_core::{Number, Real, Value};
use prima_jit::ir::{CmpOp, ElemType, FloatOp, IntOp, IrFunction, IrOp, ScalarType, SlotType};
use prima_syntax::ast::{
    AssignOp, BinOp, Block, Expr, ExprKind, IndexItem, Literal, Param, Pattern, Stmt, UnOp,
};

use crate::config::OptLevel;
use crate::eval::{Evaluator, JitFnCache};

/// A lowered value: a scalar in a slot, or an array in a slot (element type may still be unknown
/// for a not-yet-populated `[]` literal).
#[derive(Clone, Copy)]
enum Val {
    Scalar(u16, ScalarType),
    Array(u16),
}

impl Val {
    fn slot(self) -> u16 {
        match self {
            Val::Scalar(s, _) | Val::Array(s) => s,
        }
    }

    fn scalar(self) -> Option<(u16, ScalarType)> {
        match self {
            Val::Scalar(s, t) => Some((s, t)),
            Val::Array(..) => None,
        }
    }
}

/// Lower a `fn` body, or `None` when it is outside the JIT subset.
pub(crate) fn lower_fn(params: &[Param], body: &Block) -> Option<IrFunction> {
    let mut lw = Lowerer {
        ops: Vec::new(),
        slot_types: Vec::new(),
        scopes: vec![HashMap::new()],
        returns: Vec::new(),
    };
    let mut param_types = Vec::with_capacity(params.len());
    for p in params {
        if p.is_self {
            return None;
        }
        let ty = param_scalar(p.type_ann.as_ref())?;
        let slot = lw.alloc(Some(SlotType::Scalar(ty)));
        lw.bind(&p.name.value, slot);
        param_types.push(ty);
    }
    lw.block(body, true)?;

    let ret = *lw.returns.first()?;
    if lw.returns.iter().any(|&t| t != ret) {
        return None;
    }
    let slots: Option<Vec<SlotType>> = lw.slot_types.iter().copied().collect();
    Some(IrFunction {
        ops: lw.ops,
        slots: slots?,
        params: param_types,
        ret,
    })
}

/// Map a parameter annotation to a JIT scalar type.
fn param_scalar(ty: Option<&prima_syntax::ast::Type>) -> Option<ScalarType> {
    use prima_syntax::ast::Type;
    match ty? {
        Type::Integer
        | Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::I128
        | Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::U128
        | Type::Isize
        | Type::Usize => Some(ScalarType::I64),
        Type::F64 | Type::F32 => Some(ScalarType::F64),
        Type::Bool => Some(ScalarType::Bool),
        _ => None,
    }
}

struct Lowerer {
    ops: Vec<IrOp>,
    slot_types: Vec<Option<SlotType>>,
    scopes: Vec<HashMap<String, u16>>,
    returns: Vec<ScalarType>,
}

impl Lowerer {
    fn alloc(&mut self, ty: Option<SlotType>) -> u16 {
        let slot = self.slot_types.len() as u16;
        self.slot_types.push(ty);
        slot
    }

    fn set_type(&mut self, slot: u16, ty: SlotType) {
        self.slot_types[slot as usize] = Some(ty);
    }

    fn bind(&mut self, name: &str, slot: u16) {
        self.scopes
            .last_mut()
            .expect("scope")
            .insert(name.to_string(), slot);
    }

    fn lookup(&self, name: &str) -> Option<u16> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn var_type(&self, slot: u16) -> Option<SlotType> {
        self.slot_types[slot as usize]
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn emit(&mut self, op: IrOp) -> u16 {
        let dst = match &op {
            IrOp::ConstI64 { dst, .. }
            | IrOp::ConstF64 { dst, .. }
            | IrOp::ConstBool { dst, .. }
            | IrOp::Copy { dst, .. }
            | IrOp::IntBin { dst, .. }
            | IrOp::IntBinImm { dst, .. }
            | IrOp::FloatBin { dst, .. }
            | IrOp::FloatBinImm { dst, .. }
            | IrOp::IntNeg { dst, .. }
            | IrOp::FloatNeg { dst, .. }
            | IrOp::I64ToF64 { dst, .. }
            | IrOp::IntCmp { dst, .. }
            | IrOp::FloatCmp { dst, .. }
            | IrOp::BoolNot { dst, .. }
            | IrOp::NewArray { dst, .. }
            | IrOp::ArrayLen { dst, .. }
            | IrOp::ArrayGet { dst, .. } => *dst,
            _ => 0,
        };
        self.ops.push(op);
        dst
    }

    fn branch(&mut self, cond: u16, negate: bool) -> usize {
        let at = self.ops.len();
        self.ops.push(IrOp::BranchBool {
            cond,
            negate,
            target: 0,
        });
        at
    }

    fn jump(&mut self) -> usize {
        let at = self.ops.len();
        self.ops.push(IrOp::Jump { target: 0 });
        at
    }

    fn patch(&mut self, at: usize, target: usize) {
        match &mut self.ops[at] {
            IrOp::BranchBool { target: t, .. } => *t = target as u32,
            IrOp::Jump { target: t } => *t = target as u32,
            _ => unreachable!("patch target is a branch/jump"),
        }
    }

    fn ret_here(&mut self, v: Val) -> Option<()> {
        let (slot, ty) = v.scalar()?;
        self.returns.push(ty);
        self.ops.push(IrOp::Return { src: slot });
        Some(())
    }

    // ————— statements —————

    fn block(&mut self, block: &Block, tail: bool) -> Option<()> {
        let n = block.stmts.len();
        for (i, s) in block.stmts.iter().enumerate() {
            self.stmt(s, tail && i + 1 == n)?;
        }
        if n == 0 && tail {
            return None;
        }
        Some(())
    }

    fn stmt(&mut self, s: &Stmt, tail: bool) -> Option<()> {
        match s {
            Stmt::Let { pat, value, .. } => {
                let v = self.expr(value)?;
                self.bind_pat(pat, v)
            }
            Stmt::Const { name, value, .. } => {
                let v = self.expr(value)?;
                self.bind(&name.value, v.slot());
                Some(())
            }
            Stmt::Assign {
                target, op, value, ..
            } => self.assign(target, *op, value),
            Stmt::Expr(e) => {
                if tail {
                    let v = self.expr(e)?;
                    self.ret_here(v)
                } else {
                    self.expr_stmt(e)
                }
            }
            Stmt::Return { value, .. } => {
                let v = self.expr(value.as_ref()?)?;
                self.ret_here(v)
            }
            Stmt::If {
                cond,
                then,
                elifs,
                else_,
                ..
            } => self.lower_if(cond, then, elifs, else_.as_ref()),
            Stmt::While { cond, body, .. } => self.lower_while(cond, body),
            Stmt::For {
                var,
                range,
                step,
                body,
                ..
            } => self.lower_for(var, &range.0, &range.1, step.as_ref(), body),
            _ => None,
        }
    }

    /// A statement-position expression: `local.push(v)` (discarded), `local.len()` (discarded), or
    /// any pure expression (discarded). Calls with unknown effects make the lowering bail.
    fn expr_stmt(&mut self, e: &Expr) -> Option<()> {
        if let ExprKind::MethodCall {
            receiver,
            name,
            args,
        } = &e.kind
        {
            match name.value.as_str() {
                "push" => {
                    let arr = self.array_of(receiver)?;
                    let arg = args.first()?;
                    self.push_into(arr, arg)?;
                    return Some(());
                }
                "len" => {
                    let (arr, _) = self.array_of(receiver)?;
                    let dst = self.alloc(Some(SlotType::Scalar(ScalarType::I64)));
                    self.emit(IrOp::ArrayLen { dst, arr });
                    return Some(());
                }
                _ => return None,
            }
        }
        // A bare expression with no observable effect.
        let _ = self.expr(e)?;
        Some(())
    }

    fn bind_pat(&mut self, pat: &Pattern, v: Val) -> Option<()> {
        match pat {
            Pattern::Binding(name) => {
                self.bind(&name.value, v.slot());
                Some(())
            }
            Pattern::Wildcard(_) => Some(()),
            _ => None,
        }
    }

    fn assign(&mut self, target: &Expr, op: AssignOp, value: &Expr) -> Option<()> {
        match &target.kind {
            ExprKind::Path { segments } if segments.len() == 1 => {
                let slot = self.lookup(&segments[0].value)?;
                let ty = slot_scalar_type(self.var_type(slot)?)?;
                let v = self.expr(value)?.scalar()?;
                let (src, _) = self.coerce(v, ty)?;
                match op {
                    AssignOp::Assign => {
                        self.emit(IrOp::Copy { dst: slot, src });
                    }
                    AssignOp::AddAssign | AssignOp::SubAssign => {
                        let iop = if op == AssignOp::AddAssign {
                            IntOp::Add
                        } else {
                            IntOp::Sub
                        };
                        let fop = if op == AssignOp::AddAssign {
                            FloatOp::Add
                        } else {
                            FloatOp::Sub
                        };
                        self.emit_bin_into(slot, slot, src, ty, iop, fop);
                    }
                }
                Some(())
            }
            ExprKind::Index { base, index } => {
                let arr = self.array_of(base)?;
                let elem = arr.1?;
                if index.items.len() != 1 {
                    return None;
                }
                let IndexItem::Elem(ie) = &index.items[0] else {
                    return None;
                };
                let idx = self.expr(ie)?.scalar()?;
                let (idx_slot, _) = self.coerce(idx, ScalarType::I64)?;
                if op != AssignOp::Assign {
                    return None;
                }
                let v = self.expr(value)?;
                match (v, elem) {
                    (Val::Scalar(src, sty), et) => {
                        let (src, _) = self.coerce((src, sty), elem_scalar(et))?;
                        self.emit(IrOp::ArraySet {
                            arr: arr.0,
                            idx: idx_slot,
                            src,
                        });
                        Some(())
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn lower_if(
        &mut self,
        cond: &Expr,
        then: &Block,
        elifs: &[(Expr, Block)],
        else_: Option<&Block>,
    ) -> Option<()> {
        let (c, cty) = self.expr(cond)?.scalar()?;
        if cty != ScalarType::Bool {
            return None;
        }
        let else_branch = self.branch(c, true);
        self.push_scope();
        self.block(then, false)?;
        self.pop_scope();
        let end_jump = self.jump();
        self.patch(else_branch, self.ops.len());
        match elifs.split_first() {
            Some(((c2, b2), rest)) => self.lower_if(c2, b2, rest, else_)?,
            None => {
                if let Some(e) = else_ {
                    self.push_scope();
                    self.block(e, false)?;
                    self.pop_scope();
                }
            }
        }
        self.patch(end_jump, self.ops.len());
        Some(())
    }

    fn lower_while(&mut self, cond: &Expr, body: &Block) -> Option<()> {
        let header = self.ops.len();
        let (c, cty) = self.expr(cond)?.scalar()?;
        if cty != ScalarType::Bool {
            return None;
        }
        let exit = self.branch(c, true);
        self.push_scope();
        self.block(body, false)?;
        self.pop_scope();
        self.ops.push(IrOp::Jump {
            target: header as u32,
        });
        let end = self.ops.len();
        self.patch(exit, end);
        Some(())
    }

    fn lower_for(
        &mut self,
        var: &prima_syntax::ast::Spanned<String>,
        lo: &Expr,
        hi: &Expr,
        step: Option<&Expr>,
        body: &Block,
    ) -> Option<()> {
        self.push_scope();
        let (lo_slot, lo_ty) = self.expr(lo)?.scalar()?;
        let (lo_slot, _) = self.coerce((lo_slot, lo_ty), ScalarType::I64)?;
        let i = self.alloc(Some(SlotType::Scalar(ScalarType::I64)));
        self.emit(IrOp::Copy {
            dst: i,
            src: lo_slot,
        });
        self.bind(&var.value, i);
        let header = self.ops.len();
        let (hi_slot, hi_ty) = self.expr(hi)?.scalar()?;
        let (hi_slot, _) = self.coerce((hi_slot, hi_ty), ScalarType::I64)?;
        let c = self.alloc(Some(SlotType::Scalar(ScalarType::Bool)));
        self.emit(IrOp::IntCmp {
            dst: c,
            a: i,
            b: hi_slot,
            op: CmpOp::Lt,
        });
        let exit = self.branch(c, true);
        self.block(body, false)?;
        match step {
            None => {
                self.emit(IrOp::IntBinImm {
                    dst: i,
                    a: i,
                    v: 1,
                    op: IntOp::Add,
                });
            }
            Some(s) => {
                let (sv, sty) = self.expr(s)?.scalar()?;
                let (sv, _) = self.coerce((sv, sty), ScalarType::I64)?;
                self.emit(IrOp::IntBin {
                    dst: i,
                    a: i,
                    b: sv,
                    op: IntOp::Add,
                });
            }
        };
        self.ops.push(IrOp::Jump {
            target: header as u32,
        });
        let end = self.ops.len();
        self.patch(exit, end);
        self.pop_scope();
        Some(())
    }

    // ————— expressions —————

    /// Lower an array-valued expression (a variable bound to an array or an array literal).
    fn array_of(&mut self, e: &Expr) -> Option<(u16, Option<ElemType>)> {
        match &e.kind {
            ExprKind::Path { segments } if segments.len() == 1 => {
                let slot = self.lookup(&segments[0].value)?;
                match self.slot_types[slot as usize] {
                    Some(SlotType::Array(et)) => Some((slot, Some(et))),
                    Some(SlotType::Scalar(_)) => None,
                    // An unresolved `[]` literal: the element type is set by the first push/fill.
                    None => Some((slot, None)),
                }
            }
            ExprKind::Array(items) => self.array_literal(items),
            _ => None,
        }
    }

    fn array_literal(&mut self, items: &[Expr]) -> Option<(u16, Option<ElemType>)> {
        let slot = self.alloc(None);
        self.emit(IrOp::NewArray {
            dst: slot,
            elem: ElemType::F64,
        });
        for it in items {
            self.push_into((slot, None), it)?;
        }
        let et = match self.slot_types[slot as usize] {
            Some(SlotType::Array(et)) => Some(et),
            _ => None,
        };
        Some((slot, et))
    }

    /// Push `arg` onto array `arr`, resolving the array's element type on first use.
    fn push_into(&mut self, arr: (u16, Option<ElemType>), arg: &Expr) -> Option<()> {
        let (slot, known) = arr;
        // Immediate forms for literal arguments.
        if let Some(v) = literal_value(arg) {
            match v {
                LitVal::I64(x) => {
                    let et = self.resolve_elem(slot, known, ElemType::I64)?;
                    debug_assert_eq!(et, ElemType::I64);
                    self.ops.push(IrOp::ArrayPushI64Imm { arr: slot, v: x });
                    return Some(());
                }
                LitVal::F64(x) => {
                    let et = self.resolve_elem(slot, known, ElemType::F64)?;
                    debug_assert_eq!(et, ElemType::F64);
                    self.ops.push(IrOp::ArrayPushF64Imm { arr: slot, v: x });
                    return Some(());
                }
                LitVal::Bool(x) => {
                    let et = self.resolve_elem(slot, known, ElemType::Bool)?;
                    debug_assert_eq!(et, ElemType::Bool);
                    self.ops.push(IrOp::ArrayPushBoolImm { arr: slot, v: x });
                    return Some(());
                }
            }
        }
        let (src, sty) = self.expr(arg)?.scalar()?;
        let et = match sty {
            ScalarType::I64 => ElemType::I64,
            ScalarType::F64 => ElemType::F64,
            ScalarType::Bool => ElemType::Bool,
        };
        self.resolve_elem(slot, known, et)?;
        self.ops.push(IrOp::ArrayPush { arr: slot, src });
        Some(())
    }

    fn resolve_elem(
        &mut self,
        slot: u16,
        known: Option<ElemType>,
        elem: ElemType,
    ) -> Option<ElemType> {
        match known {
            Some(e) if e != elem => None,
            Some(e) => Some(e),
            None => {
                self.set_type(slot, SlotType::Array(elem));
                Some(elem)
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> Option<Val> {
        match &e.kind {
            ExprKind::Literal(lit) => self.literal(lit),
            ExprKind::Path { segments } if segments.len() == 1 => {
                let name = &segments[0].value;
                if let Some(slot) = self.lookup(name) {
                    return Some(match self.slot_types[slot as usize] {
                        Some(SlotType::Scalar(t)) => Val::Scalar(slot, t),
                        Some(SlotType::Array(_)) => Val::Array(slot),
                        None => Val::Array(slot),
                    });
                }
                // Built-in constants.
                let c = match name.as_str() {
                    "true" => Some(LitVal::Bool(true)),
                    "false" => Some(LitVal::Bool(false)),
                    "pi" => Some(LitVal::F64(std::f64::consts::PI)),
                    "e" => Some(LitVal::F64(std::f64::consts::E)),
                    "tau" => Some(LitVal::F64(std::f64::consts::TAU)),
                    _ => None,
                };
                c.map(|v| self.lit_val(v))
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs),
            ExprKind::Unary { op, operand } => self.unary(*op, operand),
            ExprKind::Call { callee, args } => self.call(callee, args),
            ExprKind::MethodCall {
                receiver,
                name,
                args,
            } => {
                // `local.len()` is the only array method usable as a value.
                if name.value == "len" && args.is_empty() {
                    let arr = self.array_of(receiver)?;
                    let dst = self.alloc(Some(SlotType::Scalar(ScalarType::I64)));
                    self.emit(IrOp::ArrayLen { dst, arr: arr.0 });
                    return Some(Val::Scalar(dst, ScalarType::I64));
                }
                None
            }
            ExprKind::Index { base, index } => {
                let arr = self.array_of(base)?;
                let et = arr.1?;
                if index.items.len() != 1 {
                    return None;
                }
                let IndexItem::Elem(ie) = &index.items[0] else {
                    return None;
                };
                let (idx, ity) = self.expr(ie)?.scalar()?;
                let (idx, _) = self.coerce((idx, ity), ScalarType::I64)?;
                let dst = self.alloc(Some(SlotType::Scalar(elem_scalar(et))));
                self.emit(IrOp::ArrayGet {
                    dst,
                    arr: arr.0,
                    idx,
                });
                Some(Val::Scalar(dst, elem_scalar(et)))
            }
            ExprKind::Array(items) => {
                let (slot, _) = self.array_literal(items)?;
                Some(Val::Array(slot))
            }
            _ => None,
        }
    }

    fn literal(&mut self, lit: &Literal) -> Option<Val> {
        match lit {
            Literal::Integer(text) => {
                let v = text.parse::<i64>().ok()?;
                Some(self.lit_val(LitVal::I64(v)))
            }
            Literal::Float(text) => {
                let v = text.parse::<f64>().ok()?;
                Some(self.lit_val(LitVal::F64(v)))
            }
            Literal::Bool(b) => Some(self.lit_val(LitVal::Bool(*b))),
            _ => None,
        }
    }

    fn lit_val(&mut self, v: LitVal) -> Val {
        let (ty, op) = match v {
            LitVal::I64(x) => (ScalarType::I64, IrOp::ConstI64 { dst: 0, v: x }),
            LitVal::F64(x) => (ScalarType::F64, IrOp::ConstF64 { dst: 0, v: x }),
            LitVal::Bool(x) => (ScalarType::Bool, IrOp::ConstBool { dst: 0, v: x }),
        };
        let dst = self.alloc(Some(SlotType::Scalar(ty)));
        let op = match op {
            IrOp::ConstI64 { v, .. } => IrOp::ConstI64 { dst, v },
            IrOp::ConstF64 { v, .. } => IrOp::ConstF64 { dst, v },
            IrOp::ConstBool { v, .. } => IrOp::ConstBool { dst, v },
            _ => unreachable!(),
        };
        self.ops.push(op);
        Val::Scalar(dst, ty)
    }

    fn unary(&mut self, op: UnOp, operand: &Expr) -> Option<Val> {
        let (src, ty) = self.expr(operand)?.scalar()?;
        let dst = self.alloc(Some(SlotType::Scalar(ty)));
        let op = match (op, ty) {
            (UnOp::Neg, ScalarType::I64) => IrOp::IntNeg { dst, src },
            (UnOp::Neg, ScalarType::F64) => IrOp::FloatNeg { dst, src },
            (UnOp::Not, ScalarType::Bool) => IrOp::BoolNot { dst, src },
            (UnOp::Pos, _) => IrOp::Copy { dst, src },
            _ => return None,
        };
        self.ops.push(op);
        Some(Val::Scalar(dst, ty))
    }

    fn binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Option<Val> {
        let (a, at) = self.expr(lhs)?.scalar()?;
        let (b, bt) = self.expr(rhs)?.scalar()?;
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Mod => {
                let iop = match op {
                    BinOp::Add => IntOp::Add,
                    BinOp::Sub => IntOp::Sub,
                    BinOp::Mul => IntOp::Mul,
                    _ => IntOp::Rem,
                };
                let fop = match op {
                    BinOp::Add => FloatOp::Add,
                    BinOp::Sub => FloatOp::Sub,
                    BinOp::Mul => FloatOp::Mul,
                    _ => FloatOp::Rem,
                };
                let ty = numeric_result(at, bt)?;
                let dst = self.alloc(Some(SlotType::Scalar(ty)));
                match ty {
                    ScalarType::I64 => self.ops.push(IrOp::IntBin { dst, a, b, op: iop }),
                    ScalarType::F64 => {
                        let (a, _) = self.coerce((a, at), ScalarType::F64)?;
                        let (b, _) = self.coerce((b, bt), ScalarType::F64)?;
                        self.ops.push(IrOp::FloatBin { dst, a, b, op: fop });
                    }
                    ScalarType::Bool => return None,
                }
                Some(Val::Scalar(dst, ty))
            }
            BinOp::Div => {
                // Only float division is representable (exact integer division yields a Rational).
                let ty = numeric_result(at, bt)?;
                if ty != ScalarType::F64 {
                    return None;
                }
                let (a, _) = self.coerce((a, at), ScalarType::F64)?;
                let (b, _) = self.coerce((b, bt), ScalarType::F64)?;
                let dst = self.alloc(Some(SlotType::Scalar(ScalarType::F64)));
                self.ops.push(IrOp::FloatBin {
                    dst,
                    a,
                    b,
                    op: FloatOp::Div,
                });
                Some(Val::Scalar(dst, ScalarType::F64))
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let cop = match op {
                    BinOp::Eq => CmpOp::Eq,
                    BinOp::Ne => CmpOp::Ne,
                    BinOp::Lt => CmpOp::Lt,
                    BinOp::Le => CmpOp::Le,
                    BinOp::Gt => CmpOp::Gt,
                    _ => CmpOp::Ge,
                };
                let ty = numeric_result(at, bt)?;
                let dst = self.alloc(Some(SlotType::Scalar(ScalarType::Bool)));
                match ty {
                    ScalarType::I64 => {
                        let (a, _) = self.coerce((a, at), ScalarType::I64)?;
                        let (b, _) = self.coerce((b, bt), ScalarType::I64)?;
                        self.ops.push(IrOp::IntCmp { dst, a, b, op: cop });
                    }
                    ScalarType::F64 => {
                        let (a, _) = self.coerce((a, at), ScalarType::F64)?;
                        let (b, _) = self.coerce((b, bt), ScalarType::F64)?;
                        self.ops.push(IrOp::FloatCmp { dst, a, b, op: cop });
                    }
                    ScalarType::Bool => return None,
                }
                Some(Val::Scalar(dst, ScalarType::Bool))
            }
            _ => None,
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Expr]) -> Option<Val> {
        let ExprKind::Path { segments } = &callee.kind else {
            return None;
        };
        if segments.len() != 1 || segments[0].value != "to_f64" || args.len() != 1 {
            return None;
        }
        let (src, ty) = self.expr(&args[0])?.scalar()?;
        match ty {
            ScalarType::F64 => Some(Val::Scalar(src, ScalarType::F64)),
            ScalarType::I64 => {
                let dst = self.alloc(Some(SlotType::Scalar(ScalarType::F64)));
                self.ops.push(IrOp::I64ToF64 { dst, src });
                Some(Val::Scalar(dst, ScalarType::F64))
            }
            ScalarType::Bool => None,
        }
    }

    /// Emit a binary op into `dst`, coercing integer operands to `ty` when it is `F64`.
    fn emit_bin_into(
        &mut self,
        dst: u16,
        a: u16,
        b: u16,
        ty: ScalarType,
        iop: IntOp,
        fop: FloatOp,
    ) {
        match ty {
            ScalarType::I64 => self.ops.push(IrOp::IntBin { dst, a, b, op: iop }),
            ScalarType::F64 => {
                // Operands are already F64 when `ty` is F64 (the caller coerced them).
                self.ops.push(IrOp::FloatBin { dst, a, b, op: fop });
            }
            ScalarType::Bool => unreachable!("bool arithmetic is rejected"),
        }
    }

    /// Coerce `(slot, from)` to `to`, emitting an `I64ToF64` when needed.
    fn coerce(&mut self, v: (u16, ScalarType), to: ScalarType) -> Option<(u16, ScalarType)> {
        let (slot, from) = v;
        if from == to {
            return Some((slot, to));
        }
        match (from, to) {
            (ScalarType::I64, ScalarType::F64) => {
                let dst = self.alloc(Some(SlotType::Scalar(ScalarType::F64)));
                self.ops.push(IrOp::I64ToF64 { dst, src: slot });
                Some((dst, ScalarType::F64))
            }
            _ => None,
        }
    }
}

/// A literal value usable as an array-push immediate or constant.
#[derive(Clone, Copy)]
enum LitVal {
    I64(i64),
    F64(f64),
    Bool(bool),
}

fn literal_value(e: &Expr) -> Option<LitVal> {
    match &e.kind {
        ExprKind::Literal(Literal::Integer(s)) => s.parse::<i64>().ok().map(LitVal::I64),
        ExprKind::Literal(Literal::Float(s)) => s.parse::<f64>().ok().map(LitVal::F64),
        ExprKind::Literal(Literal::Bool(b)) => Some(LitVal::Bool(*b)),
        ExprKind::Path { segments } if segments.len() == 1 => match segments[0].value.as_str() {
            "true" => Some(LitVal::Bool(true)),
            "false" => Some(LitVal::Bool(false)),
            _ => None,
        },
        _ => None,
    }
}

/// The common numeric type of two scalar operands, or `None` for non-numeric/bool operands.
fn numeric_result(a: ScalarType, b: ScalarType) -> Option<ScalarType> {
    match (a, b) {
        (ScalarType::I64, ScalarType::I64) => Some(ScalarType::I64),
        (ScalarType::F64, _) | (_, ScalarType::F64) => Some(ScalarType::F64),
        _ => None,
    }
}

fn elem_scalar(e: ElemType) -> ScalarType {
    match e {
        ElemType::I64 => ScalarType::I64,
        ElemType::F64 => ScalarType::F64,
        ElemType::Bool => ScalarType::Bool,
    }
}

fn slot_scalar_type(t: SlotType) -> Option<ScalarType> {
    match t {
        SlotType::Scalar(t) => Some(t),
        SlotType::Array(_) => None,
    }
}

impl Evaluator {
    /// Try to run a host `fn` through the whole-function JIT (spec §19.2). Returns `None` when the
    /// JIT is disabled, the body is outside the numeric subset, an argument is incompatible, or the
    /// compiled code deopted (exact-arithmetic overflow / out-of-range index); the caller then uses
    /// the VM/AST path. The JIT is pure, so a deopt re-run cannot duplicate side effects.
    pub(crate) fn try_jit_call(
        &self,
        params: &[Param],
        body: &Block,
        cache: &JitFnCache,
        args: &[Value],
    ) -> Option<Value> {
        if self.current_config().opt_level < OptLevel::O2 || args.len() != params.len() {
            return None;
        }
        let compiled =
            cache.get_or_init(|| lower_fn(params, body).and_then(|ir| prima_jit::compile_ir(&ir)));
        let compiled = compiled.as_ref()?;
        let mut words = Vec::with_capacity(args.len());
        for (v, ty) in args.iter().zip(&compiled.params) {
            words.push(value_to_jit_word(v, *ty)?);
        }
        let mut ctx = prima_jit::JitContext::default();
        let raw = compiled.call_raw(&mut ctx, &words);
        if ctx.error != 0 {
            return None;
        }
        Some(jit_word_to_value(raw, compiled.ret))
    }
}

/// Convert an argument `Value` to the raw word the JIT expects, or `None` when the value is not a
/// faithful representation of the parameter type (the caller then falls back to the interpreter).
fn value_to_jit_word(v: &Value, ty: ScalarType) -> Option<u64> {
    match (v, ty) {
        (Value::Number(n), ScalarType::I64) => match n {
            // Only exact integers map to the JIT's i64; a Rational/Real/etc. would change semantics.
            Number::Real(_)
            | Number::Rational(_)
            | Number::Complex { .. }
            | Number::BigFloat(_) => None,
            _ => n.as_i64().map(|x| x as u64),
        },
        (Value::Number(n), ScalarType::F64) if !n.is_complex() => Some(n.to_f64_lossy().to_bits()),
        (Value::Bool(b), ScalarType::Bool) => Some(u64::from(*b)),
        _ => None,
    }
}

/// Convert the JIT's raw result word back to a `Value`.
fn jit_word_to_value(word: u64, ty: ScalarType) -> Value {
    match ty {
        ScalarType::I64 => Value::Number(Number::from(word as i64)),
        ScalarType::F64 => Value::Number(Number::Real(Real::F64(f64::from_bits(word)))),
        ScalarType::Bool => Value::Bool(word != 0),
    }
}

#[cfg(test)]
mod jit_lower_tests {
    use super::*;
    use prima_syntax::ast::Stmt;

    fn first_fn(src: &str) -> Option<(Vec<Param>, Block)> {
        let prog = prima_syntax::parse(src).ok()?;
        for stmt in &prog.stmts {
            let inner = match stmt {
                Stmt::Pub(i) => i.as_ref(),
                other => other,
            };
            if let Stmt::FnDef { params, body, .. } = inner {
                return Some((params.clone(), body.clone()));
            }
        }
        None
    }

    #[test]
    fn lower_kernels() {
        for (name, src) in [
            (
                "sumsq",
                include_str!("../../../benches/workloads/sumsq.pra"),
            ),
            ("pi", include_str!("../../../benches/workloads/pi.pra")),
            ("fib", include_str!("../../../benches/workloads/fib.pra")),
            (
                "sieve",
                include_str!("../../../benches/workloads/sieve.pra"),
            ),
            ("dot", include_str!("../../../benches/workloads/dot.pra")),
            ("poly", include_str!("../../../benches/workloads/poly.pra")),
        ] {
            let (params, body) = first_fn(src).expect("fn");
            let ir = lower_fn(&params, &body).unwrap_or_else(|| panic!("{name}: lowering failed"));
            assert!(
                prima_jit::compile_ir(&ir).is_some(),
                "{name}: cranelift compilation failed"
            );
        }
    }
}
