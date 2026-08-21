//! Automatic differentiation (spec §19.4 stages 2–3): forward-mode dual numbers and reverse-mode tapes
//! over a numeric scalar `ExprDAG`. Both operate on the same "numeric-scalar compilable" subset the JIT
//! compiler accepts (spec §19.2): constants, parameter symbols, `+ - * /`, powers, and the elementary
//! functions `sin/cos/tan/exp/ln/log/sqrt/abs`. Free symbols are matched against `params` by name;
//! built-in constants (`\e`, `\pi`, …) fold to their `f64` value via `BuiltinSymbols`.

use std::collections::HashMap;
use std::sync::Arc;

use num_traits::ToPrimitive;
use prima_core::expr_pool::{ExprData, ExprId, ExprPool};
use prima_core::number::Real;
use prima_core::{BuiltinSymbols, SymbolId, SymbolTable};

/// Forward-mode dual number (spec §19.4 stage 2): `val` + `der` (the derivative w.r.t. one seed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dual {
    pub val: f64,
    pub der: f64,
}

#[allow(clippy::should_implement_trait)]
impl Dual {
    pub fn new(val: f64, der: f64) -> Dual {
        Dual { val, der }
    }

    /// Dual addition: `(u + u'ε) + (v + v'ε)`.
    pub fn add(self, o: Dual) -> Dual {
        Dual { val: self.val + o.val, der: self.der + o.der }
    }

    /// Dual subtraction.
    pub fn sub(self, o: Dual) -> Dual {
        Dual { val: self.val - o.val, der: self.der - o.der }
    }

    /// Dual multiplication (product rule): `(uv)' = u'v + uv'`.
    pub fn mul(self, o: Dual) -> Dual {
        Dual { val: self.val * o.val, der: self.der * o.val + self.val * o.der }
    }

    /// Dual division: `(u/v)' = (u'v - uv')/v²`.
    pub fn div(self, o: Dual) -> Dual {
        let v2 = o.val * o.val;
        Dual { val: self.val / o.val, der: (self.der * o.val - self.val * o.der) / v2 }
    }

    /// Dual negation.
    pub fn neg(self) -> Dual {
        Dual { val: -self.val, der: -self.der }
    }

    /// Dual power `u^v`: `d/dx = v·u^(v-1)·u' + ln(u)·u^v·v'` (log-derivative for the `v` part).
    pub fn pow(self, o: Dual) -> Dual {
        let base = self.val.powf(o.val);
        Dual {
            val: base,
            der: o.val * self.val.powf(o.val - 1.0) * self.der + self.val.ln() * base * o.der,
        }
    }

    pub fn sin(self) -> Dual {
        Dual { val: self.val.sin(), der: self.der * self.val.cos() }
    }

    pub fn cos(self) -> Dual {
        Dual { val: self.val.cos(), der: -self.der * self.val.sin() }
    }

    /// sec²(x) = 1 + tan²(x).
    pub fn tan(self) -> Dual {
        let t = self.val.tan();
        Dual { val: t, der: self.der * (1.0 + t * t) }
    }

    pub fn exp(self) -> Dual {
        let e = self.val.exp();
        Dual { val: e, der: self.der * e }
    }

    pub fn ln(self) -> Dual {
        Dual { val: self.val.ln(), der: self.der / self.val }
    }

    pub fn sqrt(self) -> Dual {
        let s = self.val.sqrt();
        Dual { val: s, der: self.der / (2.0 * s) }
    }

    /// abs'(x) = sign(x).
    pub fn abs(self) -> Dual {
        Dual { val: self.val.abs(), der: self.der * self.val.signum() }
    }
}

/// Binary node kinds of the reverse-mode tape (spec §19.4 stage 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryKind {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

/// Elementary-function node kinds of the tape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathKind {
    Sin,
    Cos,
    Tan,
    Exp,
    Ln,
    Sqrt,
    Abs,
}

/// A node of the computation graph: a constant, a parameter input (index into the input vector),
/// a binary operation, or an elementary function applied to a child node.
#[derive(Debug, Clone, Copy)]
enum Node {
    Const(f64),
    Param(u8),
    Binary(BinaryKind, usize, usize),
    Math(MathKind, usize),
}

/// Reverse-mode tape (spec §19.4 stage 3): records the forward computation once (nodes in topological
/// order — a child always precedes its parents), then computes ALL partial derivatives in one backward
/// pass per input vector.
pub struct Tape {
    nodes: Vec<Node>,
    n_params: usize,
    output: usize,
    /// Node index of the (single) `Param` node per parameter, in parameter order (for collecting adjoints).
    param_nodes: Vec<usize>,
}

impl Tape {
    /// Build the tape from a numeric scalar DAG, or `None` if `expr` is not numeric-compilable
    /// (a non-param/non-constant symbol, a multi-argument application, an unknown function, …).
    pub fn build(pool: &ExprPool, builtins: &BuiltinSymbols, expr: ExprId, params: &[String]) -> Option<Arc<Tape>> {
        let symbols = SymbolTable::global();
        // Parameter symbols map by name; built-in constants fold to their f64 value.
        let mut param_ids: HashMap<SymbolId, u8> = HashMap::new();
        for (i, name) in params.iter().enumerate() {
            if i > u8::MAX as usize {
                return None;
            }
            param_ids.insert(symbols.intern(name), i as u8);
        }
        let mut consts: HashMap<SymbolId, f64> = HashMap::new();
        consts.insert(builtins.e, std::f64::consts::E);
        consts.insert(builtins.pi, std::f64::consts::PI);
        consts.insert(builtins.tau, std::f64::consts::TAU);
        consts.insert(builtins.gamma, 0.577_215_664_901_532_9);
        consts.insert(builtins.phi, 1.618_033_988_749_895);

        let mut builder = Builder {
            pool,
            builtins,
            param_ids,
            consts,
            memo: HashMap::new(),
            nodes: Vec::new(),
            param_nodes: vec![usize::MAX; params.len()],
        };
        let output = builder.build_node(expr)?;
        if builder.param_nodes.contains(&usize::MAX) {
            // A parameter never reached: the expression does not depend on it (derivative 0).
            // The forward pass still works, but `grad` needs a node to read the zero adjoint from —
            // a constant `0` node is a safe stand-in that never receives adjoints.
            for slot in builder.param_nodes.iter_mut() {
                if *slot == usize::MAX {
                    *slot = builder.nodes.len();
                    builder.nodes.push(Node::Const(0.0));
                }
            }
        }
        Some(Arc::new(Tape { nodes: builder.nodes, n_params: params.len(), output, param_nodes: builder.param_nodes }))
    }

    /// Gradient of the expression w.r.t. all parameters (`len == params.len()`): one forward pass to
    /// compute every node's value, then one backward pass seeding `adj[output] = 1`.
    pub fn grad(&self, inputs: &[f64]) -> Vec<f64> {
        debug_assert_eq!(inputs.len(), self.n_params, "grad input arity mismatch");
        let mut vals = vec![0.0f64; self.nodes.len()];
        let mut adj = vec![0.0f64; self.nodes.len()];
        for (i, node) in self.nodes.iter().enumerate() {
            vals[i] = match *node {
                Node::Const(c) => c,
                Node::Param(p) => inputs[p as usize],
                Node::Binary(k, l, r) => binary_val(k, vals[l], vals[r]),
                Node::Math(k, c) => math_val(k, vals[c]),
            };
        }
        adj[self.output] = 1.0;
        for i in (0..self.nodes.len()).rev() {
            let a = adj[i];
            match self.nodes[i] {
                Node::Const(_) | Node::Param(_) => {}
                Node::Binary(k, l, r) => match k {
                    BinaryKind::Add => {
                        adj[l] += a;
                        adj[r] += a;
                    }
                    BinaryKind::Sub => {
                        adj[l] += a;
                        adj[r] -= a;
                    }
                    BinaryKind::Mul => {
                        adj[l] += a * vals[r];
                        adj[r] += a * vals[l];
                    }
                    BinaryKind::Div => {
                        adj[l] += a / vals[r];
                        adj[r] += -a * vals[l] / (vals[r] * vals[r]);
                    }
                    BinaryKind::Pow => {
                        // b = base, e = exp: ∂/∂b = e·b^(e-1), ∂/∂e = ln(b)·b^e.
                        adj[l] += a * vals[r] * vals[l].powf(vals[r] - 1.0);
                        adj[r] += a * vals[l].ln() * vals[l].powf(vals[r]);
                    }
                },
                Node::Math(k, c) => match k {
                    MathKind::Sin => adj[c] += a * vals[c].cos(),
                    MathKind::Cos => adj[c] -= a * vals[c].sin(),
                    MathKind::Tan => {
                        let t = vals[c].tan();
                        adj[c] += a * (1.0 + t * t);
                    }
                    MathKind::Exp => adj[c] += a * vals[c].exp(),
                    MathKind::Ln => adj[c] += a / vals[c],
                    MathKind::Sqrt => adj[c] += a / (2.0 * vals[c].sqrt()),
                    MathKind::Abs => adj[c] += a * vals[c].signum(),
                },
            }
        }
        // Adjoint of each parameter's node, in parameter order (spec §19.4 stage 3).
        self.param_nodes.iter().map(|&n| adj[n]).collect()
    }
}

/// Post-order DFS over the DAG. The DAG is hash-consed/shared, so a memo prevents re-adding a node:
/// every unique `ExprId` becomes exactly one tape node (a child precedes its parents).
struct Builder<'a> {
    pool: &'a ExprPool,
    builtins: &'a BuiltinSymbols,
    param_ids: HashMap<SymbolId, u8>,
    consts: HashMap<SymbolId, f64>,
    memo: HashMap<ExprId, usize>,
    nodes: Vec<Node>,
    /// Node index of the `Param` node per parameter (filled on first occurrence).
    param_nodes: Vec<usize>,
}

impl Builder<'_> {
    fn build_node(&mut self, id: ExprId) -> Option<usize> {
        if let Some(&idx) = self.memo.get(&id) {
            return Some(idx);
        }
        let idx = match self.pool.get(id)? {
            ExprData::Integer(i) => self.push(Node::Const(i.to_f64()?)),
            ExprData::Rational(r) => self.push(Node::Const(r.to_f64()?)),
            ExprData::Real(Real::F32(f)) => self.push(Node::Const(f as f64)),
            ExprData::Real(Real::F64(f)) => self.push(Node::Const(f)),
            ExprData::Symbol(s) => {
                if let Some(&p) = self.param_ids.get(&s) {
                    let idx = self.push(Node::Param(p));
                    self.param_nodes[p as usize] = idx;
                    idx
                } else if let Some(&c) = self.consts.get(&s) {
                    self.push(Node::Const(c))
                } else {
                    return None;
                }
            }
            ExprData::Add(items) => self.push_binary_chain(BinaryKind::Add, &items)?,
            ExprData::Mul(items) => self.push_binary_chain(BinaryKind::Mul, &items)?,
            ExprData::Pow { base, exp } => {
                let l = self.build_node(base)?;
                let r = self.build_node(exp)?;
                self.push(Node::Binary(BinaryKind::Pow, l, r))
            }
            ExprData::Apply { f, args } => {
                if args.len() != 1 {
                    return None;
                }
                let child = self.build_node(args[0])?;
                let ExprData::Symbol(sym) = self.pool.get(f)? else { return None };
                let kind = math_kind(self.builtins, sym)?;
                self.push(Node::Math(kind, child))
            }
            ExprData::Indeterminate(_) => return None,
        };
        self.memo.insert(id, idx);
        Some(idx)
    }

    /// Left-associative chain for an n-ary `Add`/`Mul` list (the DAG stores them flat).
    fn push_binary_chain(&mut self, kind: BinaryKind, items: &[ExprId]) -> Option<usize> {
        let mut acc = self.build_node(items[0])?;
        for &it in &items[1..] {
            let r = self.build_node(it)?;
            acc = self.push(Node::Binary(kind, acc, r));
        }
        Some(acc)
    }

    fn push(&mut self, node: Node) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }
}

/// Map an applied function symbol to its tape node kind; `None` for non-math functions.
fn math_kind(builtins: &BuiltinSymbols, f: SymbolId) -> Option<MathKind> {
    if f == builtins.sin {
        Some(MathKind::Sin)
    } else if f == builtins.cos {
        Some(MathKind::Cos)
    } else if f == builtins.tan {
        Some(MathKind::Tan)
    } else if f == builtins.exp {
        Some(MathKind::Exp)
    } else if f == builtins.ln || f == builtins.log {
        // `log` is evaluated as the natural log in the runtime (spec §8.3 constant folding), so both map to `ln`.
        Some(MathKind::Ln)
    } else if f == builtins.sqrt {
        Some(MathKind::Sqrt)
    } else if f == builtins.abs {
        Some(MathKind::Abs)
    } else {
        None
    }
}

fn binary_val(k: BinaryKind, l: f64, r: f64) -> f64 {
    match k {
        BinaryKind::Add => l + r,
        BinaryKind::Sub => l - r,
        BinaryKind::Mul => l * r,
        BinaryKind::Div => l / r,
        BinaryKind::Pow => l.powf(r),
    }
}

fn math_val(k: MathKind, x: f64) -> f64 {
    match k {
        MathKind::Sin => x.sin(),
        MathKind::Cos => x.cos(),
        MathKind::Tan => x.tan(),
        MathKind::Exp => x.exp(),
        MathKind::Ln => x.ln(),
        MathKind::Sqrt => x.sqrt(),
        MathKind::Abs => x.abs(),
    }
}

/// Evaluate `expr` with forward-mode AD seeded on parameter `wrt` (spec §19.4 stage 2); returns the
/// derivative, or `None` if the DAG is not numeric-compilable.
pub fn forward_derivative(
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
    expr: ExprId,
    params: &[String],
    inputs: &[f64],
    wrt: usize,
) -> Option<f64> {
    if wrt >= params.len() || inputs.len() != params.len() {
        return None;
    }
    let tape = Tape::build(pool, builtins, expr, params)?;
    let mut duals = vec![Dual::new(0.0, 0.0); tape.nodes.len()];
    for (i, node) in tape.nodes.iter().enumerate() {
        duals[i] = match *node {
            Node::Const(c) => Dual::new(c, 0.0),
            Node::Param(p) => Dual::new(inputs[p as usize], if p as usize == wrt { 1.0 } else { 0.0 }),
            Node::Binary(k, l, r) => dual_binary(k, duals[l], duals[r]),
            Node::Math(k, c) => dual_math(k, duals[c]),
        };
    }
    Some(duals[tape.output].der)
}

fn dual_binary(k: BinaryKind, l: Dual, r: Dual) -> Dual {
    match k {
        BinaryKind::Add => l.add(r),
        BinaryKind::Sub => l.sub(r),
        BinaryKind::Mul => l.mul(r),
        BinaryKind::Div => l.div(r),
        BinaryKind::Pow => l.pow(r),
    }
}

fn dual_math(k: MathKind, x: Dual) -> Dual {
    match k {
        MathKind::Sin => x.sin(),
        MathKind::Cos => x.cos(),
        MathKind::Tan => x.tan(),
        MathKind::Exp => x.exp(),
        MathKind::Ln => x.ln(),
        MathKind::Sqrt => x.sqrt(),
        MathKind::Abs => x.abs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (&'static ExprPool, &'static BuiltinSymbols) {
        (ExprPool::global(), BuiltinSymbols::global())
    }

    fn p(pool: &ExprPool, name: &str) -> ExprId {
        pool.symbol(SymbolTable::global().intern(name))
    }

    fn approx(a: f64, b: f64, eps: f64) {
        assert!((a - b).abs() < eps, "expected {a} ≈ {b} within {eps}");
    }

    #[test]
    fn dual_forward_derivative_square() {
        let (pool, b) = setup();
        let params = vec!["x".to_string()];
        let expr = pool.pow2(p(pool, "x"), pool.integer(2)); // x^2
        let d = forward_derivative(pool, b, expr, &params, &[3.0], 0).expect("compilable");
        approx(d, 6.0, 1e-9);
    }

    #[test]
    fn dual_forward_derivative_sin() {
        let (pool, b) = setup();
        let params = vec!["x".to_string()];
        let expr = pool.apply(pool.symbol(b.sin), &[p(pool, "x")]); // sin(x)
        let d = forward_derivative(pool, b, expr, &params, &[0.0], 0).expect("compilable");
        approx(d, 1.0, 1e-9);
    }

    #[test]
    fn dual_forward_derivative_two_var() {
        let (pool, b) = setup();
        let params = vec!["x".to_string(), "y".to_string()];
        // x^2*y, w.r.t. x at (2, 3) → 2·2·3 = 12.
        let expr = pool.mul2(pool.pow2(p(pool, "x"), pool.integer(2)), p(pool, "y"));
        let d = forward_derivative(pool, b, expr, &params, &[2.0, 3.0], 0).expect("compilable");
        approx(d, 12.0, 1e-9);
    }

    #[test]
    fn tape_gradient_polynomial() {
        let (pool, b) = setup();
        let params = vec!["x".to_string(), "y".to_string()];
        // x^2*y + y^3, gradient at (1, 2): ∂x = 2·1·2 = 4, ∂y = 1 + 3·4 = 13.
        let expr = pool.add2(
            pool.mul2(pool.pow2(p(pool, "x"), pool.integer(2)), p(pool, "y")),
            pool.pow2(p(pool, "y"), pool.integer(3)),
        );
        let tape = Tape::build(pool, b, expr, &params).expect("compilable");
        let g = tape.grad(&[1.0, 2.0]);
        assert_eq!(g.len(), 2);
        approx(g[0], 4.0, 1e-9);
        approx(g[1], 13.0, 1e-9);
    }

    #[test]
    fn tape_gradient_sin_times_y() {
        let (pool, b) = setup();
        let params = vec!["x".to_string(), "y".to_string()];
        // sin(x)*y, gradient at (0, 2): ∂x = cos(0)·2 = 2, ∂y = sin(0) = 0.
        let expr = pool.mul2(pool.apply(pool.symbol(b.sin), &[p(pool, "x")]), p(pool, "y"));
        let tape = Tape::build(pool, b, expr, &params).expect("compilable");
        let g = tape.grad(&[0.0, 2.0]);
        approx(g[0], 2.0, 1e-9);
        approx(g[1], 0.0, 1e-9);
    }

    #[test]
    fn tape_uses_builtin_constants() {
        let (pool, b) = setup();
        let params = vec!["x".to_string()];
        // sin(pi * x) at x = 0.5: ∂x = cos(pi/2)·pi = 0.
        let expr = pool.apply(pool.symbol(b.sin), &[pool.mul2(pool.symbol(b.pi), p(pool, "x"))]);
        let tape = Tape::build(pool, b, expr, &params).expect("compilable");
        let g = tape.grad(&[0.5]);
        approx(g[0], 0.0, 1e-9);
    }

    #[test]
    fn tape_rejects_non_numeric_dag() {
        let (pool, b) = setup();
        let params = vec!["x".to_string()];
        // `f(x)` applied to a symbol that is not a param and not a builtin constant → None.
        let unknown = pool.symbol(SymbolTable::global().intern("z"));
        let expr = pool.apply(unknown, &[p(pool, "x")]);
        assert!(Tape::build(pool, b, expr, &params).is_none());
    }

    #[test]
    fn tape_shared_subexpression_memoizes() {
        let (pool, b) = setup();
        let params = vec!["x".to_string()];
        // x^3 = x*x*x as a chain — the DAG is hash-consed so the same `x` node appears once.
        let x = p(pool, "x");
        let x3 = pool.mul2(pool.mul2(x, x), x);
        let tape = Tape::build(pool, b, x3, &params).expect("compilable");
        let g = tape.grad(&[2.0]);
        approx(g[0], 12.0, 1e-9);
    }
}
