//! Prima JIT (spec §19.2): hot-path compilation of numeric scalar expressions to native code with
//! cranelift. The symbol layer stays interpreted; only functions whose bodies are numeric scalar
//! expressions (no control flow, no side effects) are compiled.
//!
//! Pipeline: `ExprDAG → Bytecode → cranelift IR → native code` (implementation plan §5). The bytecode
//! is a tiny stack machine over `f64`; transcendental functions are lowered to calls of registered
//! Rust trampolines so the generated code never depends on the platform's libm symbol names.

pub mod bytecode;
pub mod compiler;
pub mod engine;

pub use bytecode::{Bytecode, Op};
pub use compiler::{compile_scalar, dag_to_bytecode};
pub use engine::{CompiledScalar, compile_bytecode};
