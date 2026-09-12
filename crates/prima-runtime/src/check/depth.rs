//! Static-checker recursion budget (spec §16.4): bounds the recursive AST walks (`infer`,
//! `collect_expr_errors`) so pathological nesting cannot overflow the stack. Defense in depth —
//! the parser already rejects source whose AST exceeds its own budget — shared via a thread-local
//! counter with a `Drop`-safe guard so panics and early returns cannot leave it unbalanced.

use std::cell::Cell;

/// Maximum AST depth the checker walks (spec §16.4). Checker frames are small (≈100–200 B), so
/// 2 000 levels use well under 1 MB of stack; it matches the parser's and evaluator's budgets.
pub(crate) const MAX_CHECK_DEPTH: u32 = 2_000;

thread_local! {
    static WALK_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// A held walk-depth slot; decrements the counter on drop.
pub(crate) struct WalkDepthGuard;

impl Drop for WalkDepthGuard {
    fn drop(&mut self) {
        WALK_DEPTH.with(|d| d.set(d.get() - 1));
    }
}

/// Enter one AST-walk level; `None` when the budget is exhausted (the caller must stop descending
/// and report/skip, never recurse further).
pub(crate) fn enter_walk_depth() -> Option<WalkDepthGuard> {
    WALK_DEPTH.with(|d| {
        let n = d.get() + 1;
        if n > MAX_CHECK_DEPTH {
            return None;
        }
        d.set(n);
        Some(WalkDepthGuard)
    })
}
