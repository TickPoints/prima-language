//! Bytecode VM for Prima (spec §19.5, Milestone B).
//!
//! The interpreter is extended with a stack bytecode VM: a per-function `Chunk` is produced by
//! `compiler` (AST → bytecode) and executed by `exec` (a `call/return` stack machine over `Value`
//! slots). The VM is gated by the `vm` config policy (spec §13.2); when `false` the AST interpreter
//! owns execution. When a construct is not yet lowered, the VM falls back to the AST `eval_*` path,
//! so the VM can be extended incrementally without ever changing observable behavior.

pub mod comp;
pub mod exec;
pub mod op;

pub use op::{Chunk, Const, Op, Program};
