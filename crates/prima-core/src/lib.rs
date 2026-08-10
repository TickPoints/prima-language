pub mod error;
pub mod expr_pool;
pub mod number;
pub mod value;

pub use error::CoreError;
pub use expr_pool::{ExprData, ExprId, ExprPool};
pub use number::{Number, Real};
pub use value::{IndeterminateForm, Value};
