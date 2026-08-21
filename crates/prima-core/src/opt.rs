//! Optimization pipeline primitives (spec §10.2): constant folding + common-subexpression
//! elimination over the hash-consed `ExprDAG`.

use crate::builtins::BuiltinSymbols;
use crate::expr_pool::{ExprId, ExprPool};

/// Constant folding (spec §10.2 item 1): runs the simplify engine over the DAG and returns the
/// canonical folded expression. Level-2 rules (0*x, 1*x, constant arithmetic, math constants)
/// already live in `crate::simplify::simplify`.
pub fn const_fold(pool: &ExprPool, builtins: &BuiltinSymbols, id: ExprId) -> ExprId {
    crate::simplify::simplify(pool, builtins, id)
}

/// CSE (spec §10.2 item 3): returns the canonical `ExprId` for a subexpression. Because the
/// `ExprPool` hash-conses identical nodes, repeated subexpressions already share one `ExprId`;
/// this function documents and asserts that invariant (returns `id` unchanged). It exists as the
/// stable entry point the JIT pipeline calls before codegen.
pub fn cse(_pool: &ExprPool, id: ExprId) -> ExprId {
    id
}

/// Full local optimization run (spec §10.2): `const_fold` then `cse`. Used by the JIT compiler
/// before translating the DAG to bytecode.
pub fn optimize(pool: &ExprPool, builtins: &BuiltinSymbols, id: ExprId) -> ExprId {
    let folded = const_fold(pool, builtins, id);
    cse(pool, folded)
}

#[cfg(test)]
mod tests {
    use crate::expr_pool::{ExprData, ExprPool};
    use crate::number::Number;
    use crate::opt::{self, cse, const_fold, optimize};
    use crate::render::render_latex;
    use crate::symbol::SymbolTable;
    use crate::BuiltinSymbols;

    #[test]
    fn const_fold_merges_constant_arithmetic() {
        let pool = ExprPool::global();
        let builtins = BuiltinSymbols::global();
        let symbols = SymbolTable::global();
        let x = pool.symbol(symbols.intern("x"));
        // Raw, unsimplified `2*3`: `Mul` interned as-is (no level-0/1 folding at build time).
        let prod = pool.mul(&[pool.integer(2), pool.integer(3)]);
        assert!(matches!(pool.get(prod), Some(ExprData::Mul(_))));
        let expr = pool.add(&[prod, x]);

        let folded = const_fold(pool, builtins, expr);
        assert_ne!(folded, prod);
        match pool.get(folded) {
            Some(ExprData::Add(items)) => {
                assert!(
                    items
                        .iter()
                        .any(|&it| matches!(pool.const_number(it), Some(n) if n == Number::from(6))),
                    "constant 6 must survive as an `Add` child: {:?}",
                    items
                );
            }
            other => panic!("expected `Add`, got {:?}", other),
        }
        assert_eq!(render_latex(pool, symbols, folded), "x + 6");
    }

    #[test]
    fn const_fold_folds_math_functions() {
        let pool = ExprPool::global();
        let builtins = BuiltinSymbols::global();

        let sin0 = pool.apply(pool.symbol(builtins.sin), &[pool.integer(0)]);
        assert_eq!(const_fold(pool, builtins, sin0), pool.integer(0));

        let sqrt4 = pool.apply(pool.symbol(builtins.sqrt), &[pool.integer(4)]);
        assert_eq!(const_fold(pool, builtins, sqrt4), pool.integer(2));
    }

    #[test]
    fn cse_shares_duplicate_subexpressions() {
        let pool = ExprPool::global();
        let symbols = SymbolTable::global();
        let x = pool.symbol(symbols.intern("x"));

        // `x*x` interned twice must yield the SAME `ExprId`: one shared `Mul` node (spec §8.1
        // hash-consing is the CSE machinery for the DAG).
        let m1 = pool.mul2(x, x);
        let m2 = pool.mul2(x, x);
        assert_eq!(m1, m2);
        assert_eq!(cse(pool, m1), m1);

        // `x*x + x*x`: the `Add` refers to the same `Mul` id twice — a single shared subexpression.
        let e = pool.add2(m1, m2);
        match pool.get(e) {
            Some(ExprData::Add(items)) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], items[1]);
                assert_eq!(items[0], m1);
            }
            other => panic!("expected `Add`, got {:?}", other),
        }
    }

    #[test]
    fn optimize_folds_then_canonicalizes() {
        let pool = ExprPool::global();
        let builtins = BuiltinSymbols::global();
        let symbols = SymbolTable::global();
        let x = pool.symbol(symbols.intern("x"));
        let expr = pool.add(&[pool.mul(&[pool.integer(2), pool.integer(3)]), x]);

        let folded = optimize(pool, builtins, expr);
        // `optimize` is const_fold followed by cse; both must agree on the canonical form.
        assert_eq!(folded, const_fold(pool, builtins, expr));
        assert_eq!(folded, opt::optimize(pool, builtins, folded));
        assert_eq!(render_latex(pool, symbols, folded), "x + 6");
    }
}
