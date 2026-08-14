//! `linalg` module (spec §18 / appendix B.2): matrices and linear algebra over `F64`, backed by
//! `nalgebra`.
//!
//! Representation (spec §11.3): a matrix is `Value::Array` of rows, each row a `Value::Array` of
//! `Value::Number`; a vector is a flat `Value::Array` of `Value::Number`. The matrix layer is
//! numeric, so every value is collapsed to `F64` as it enters and leaves the module.

use std::collections::HashMap;

use nalgebra::{DMatrix, DVector};
use prima_core::{Number, Real, Value};
use prima_runtime::stdlib::register_namespace;
use prima_runtime::{Evaluator, Function, NamespaceItem, RuntimeError};

type Native = fn(&mut Evaluator, &[Value]) -> Result<Value, RuntimeError>;

fn native(name: &'static str, call: Native) -> NamespaceItem {
    NamespaceItem::Func(Function::Native { name, call })
}

fn arity(args: &[Value], n: usize, fname: &str) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(RuntimeError::Message(format!(
            "`{fname}` expects {n} argument(s), got {}",
            args.len()
        )))
    }
}

/// Register the `linalg` namespace (spec appendix B.2). `Matrix::*` items resolve through the
/// flattened module-item lookup (`linalg::Matrix::zeros`, see `eval::resolve_func`).
pub fn register() {
    let mut items = HashMap::new();
    items.insert("Matrix::zeros".into(), native("linalg::Matrix::zeros", matrix_zeros));
    items.insert("Matrix::ones".into(), native("linalg::Matrix::ones", matrix_ones));
    items.insert("Matrix::identity".into(), native("linalg::Matrix::identity", matrix_identity));
    items.insert("Matrix::diagonal".into(), native("linalg::Matrix::diagonal", matrix_diagonal));
    items.insert("Matrix::from_rows".into(), native("linalg::Matrix::from_rows", matrix_from_rows));
    items.insert("Matrix::from_cols".into(), native("linalg::Matrix::from_cols", matrix_from_cols));
    items.insert("transpose".into(), native("linalg::transpose", transpose));
    items.insert("inverse".into(), native("linalg::inverse", inverse));
    items.insert("determinant".into(), native("linalg::determinant", determinant));
    items.insert("trace".into(), native("linalg::trace", trace));
    items.insert("rank".into(), native("linalg::rank", rank));
    items.insert("norm".into(), native("linalg::norm", norm));
    items.insert("cond".into(), native("linalg::cond", cond));
    items.insert("dot".into(), native("linalg::dot", dot));
    items.insert("cross".into(), native("linalg::cross", cross));
    items.insert("lu".into(), native("linalg::lu", lu));
    items.insert("qr".into(), native("linalg::qr", qr));
    items.insert("svd".into(), native("linalg::svd", svd));
    items.insert("eigen".into(), native("linalg::eigen", eigen));
    items.insert("cholesky".into(), native("linalg::cholesky", cholesky));
    items.insert("solve".into(), native("linalg::solve", solve));
    items.insert("lstsq".into(), native("linalg::lstsq", lstsq));
    register_namespace("linalg", items);
}

// —— value conversion helpers (spec §11.3 representation) ——

fn scalar(x: f64) -> Value {
    Value::Number(Number::Real(Real::F64(x)))
}

/// Convert a `DMatrix` back to the nested-array `Value` representation.
fn matrix_value(m: &DMatrix<f64>) -> Value {
    Value::Array(
        (0..m.nrows())
            .map(|r| Value::Array((0..m.ncols()).map(|c| scalar(m[(r, c)])).collect()))
            .collect(),
    )
}

/// Convert a `DVector` back to the flat-array `Value` representation.
fn vector_value(v: &DVector<f64>) -> Value {
    Value::Array(v.iter().map(|&x| scalar(x)).collect())
}

fn type_err(fname: &str, msg: impl Into<String>) -> RuntimeError {
    RuntimeError::Type(format!("`{fname}`: {}", msg.into()))
}

/// Parse a `Value` as a numeric matrix (`[[a, b], [c, d]]`). Ragged rows and non-numeric elements
/// are errors (R0009/R0004); empty matrices are rejected (R0014).
fn as_matrix(v: &Value, fname: &str) -> Result<DMatrix<f64>, RuntimeError> {
    let rows = match v {
        Value::Array(rows) => rows,
        other => return Err(type_err(fname, format!("expected a numeric matrix (R0009), got {other:?}"))),
    };
    if rows.is_empty() {
        return Err(type_err(fname, "empty matrix (R0014)"));
    }
    let ncols = match &rows[0] {
        Value::Array(r) if !r.is_empty() => r.len(),
        Value::Array(_) => return Err(type_err(fname, "empty matrix row (R0014)")),
        other => return Err(type_err(fname, format!("expected a numeric matrix (R0009), got row {other:?}"))),
    };
    let mut data = Vec::with_capacity(rows.len() * ncols);
    for row in rows {
        let r = match row {
            Value::Array(r) => r,
            other => return Err(type_err(fname, format!("expected a numeric matrix (R0009), got row {other:?}"))),
        };
        if r.len() != ncols {
            return Err(type_err(fname, "ragged rows: dimension mismatch (R0004)"));
        }
        for el in r {
            match el {
                Value::Number(n) if !n.is_complex() => data.push(n.to_f64_lossy()),
                other => {
                    return Err(type_err(
                        fname,
                        format!("expected a numeric matrix (R0009), got element {other:?}"),
                    ))
                }
            }
        }
    }
    Ok(DMatrix::from_row_slice(rows.len(), ncols, &data))
}

/// Parse a `Value` as a numeric vector (flat array of `Number`).
fn as_vector(v: &Value, fname: &str) -> Result<DVector<f64>, RuntimeError> {
    let elems = match v {
        Value::Array(elems) => elems,
        other => return Err(type_err(fname, format!("expected a numeric vector (R0009), got {other:?}"))),
    };
    if elems.is_empty() {
        return Err(type_err(fname, "empty vector (R0014)"));
    }
    let mut data = Vec::with_capacity(elems.len());
    for el in elems {
        match el {
            Value::Number(n) if !n.is_complex() => data.push(n.to_f64_lossy()),
            other => {
                return Err(type_err(
                    fname,
                    format!("expected a numeric vector (R0009), got element {other:?}"),
                ))
            }
        }
    }
    Ok(DVector::from_column_slice(&data))
}

/// Parse a positive integer dimension argument (rows/cols/order).
fn dim_arg(args: &[Value], i: usize, fname: &str) -> Result<usize, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) => match usize::try_from(n.as_u64().unwrap_or(u64::MAX)) {
            Ok(x) if x >= 1 => Ok(x),
            _ => Err(RuntimeError::Message(format!(
                "`{fname}` dimension {i} must be a positive integer, got {n}"
            ))),
        },
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` dimension {i} must be an integer, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!("`{fname}` missing argument {i}"))),
    }
}

fn require_square(m: &DMatrix<f64>, fname: &str) -> Result<(), RuntimeError> {
    if m.nrows() == m.ncols() {
        Ok(())
    } else {
        Err(type_err(
            fname,
            format!("matrix must be square, got {}x{} (R0004)", m.nrows(), m.ncols()),
        ))
    }
}

/// Numeric rank: number of singular values above a scale-relative tolerance (spec appendix B.2).
fn matrix_rank(m: &DMatrix<f64>) -> usize {
    let sv = m.clone().svd(false, false).singular_values;
    let max = sv.iter().copied().fold(0.0_f64, f64::max);
    let eps = if max > 0.0 {
        max * (m.nrows().max(m.ncols()) as f64) * f64::EPSILON * 10.0
    } else {
        0.0
    };
    sv.iter().filter(|&&s| s > eps).count()
}

/// Matrix p-norm: 2 = Frobenius, 1 = max column sum, inf = max row sum (spec appendix B.2).
fn matrix_norm(m: &DMatrix<f64>, p: f64, fname: &str) -> Result<f64, RuntimeError> {
    if p == 2.0 {
        Ok(m.norm())
    } else if p == 1.0 {
        Ok((0..m.ncols())
            .map(|c| (0..m.nrows()).map(|r| m[(r, c)].abs()).sum::<f64>())
            .fold(0.0_f64, f64::max))
    } else if p.is_infinite() && p.is_sign_positive() {
        Ok((0..m.nrows())
            .map(|r| (0..m.ncols()).map(|c| m[(r, c)].abs()).sum::<f64>())
            .fold(0.0_f64, f64::max))
    } else {
        Err(RuntimeError::Message(format!(
            "`{fname}`: unsupported p-norm p={p}; supported: 1, 2, inf"
        )))
    }
}

/// Vector p-norm: 2 = Euclidean, 1 = sum of absolute values, inf = max absolute value.
fn vector_norm(v: &DVector<f64>, p: f64, fname: &str) -> Result<f64, RuntimeError> {
    if p == 2.0 {
        Ok(v.norm())
    } else if p == 1.0 {
        Ok(v.iter().map(|x| x.abs()).sum())
    } else if p.is_infinite() && p.is_sign_positive() {
        Ok(v.iter().map(|x| x.abs()).fold(0.0_f64, f64::max))
    } else {
        Err(RuntimeError::Message(format!(
            "`{fname}`: unsupported p-norm p={p}; supported: 1, 2, inf"
        )))
    }
}

// —— constructors (spec appendix B.2) ——

fn matrix_zeros(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "linalg::Matrix::zeros")?;
    let rows = dim_arg(args, 0, "linalg::Matrix::zeros")?;
    let cols = dim_arg(args, 1, "linalg::Matrix::zeros")?;
    Ok(matrix_value(&DMatrix::zeros(rows, cols)))
}

fn matrix_ones(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "linalg::Matrix::ones")?;
    let rows = dim_arg(args, 0, "linalg::Matrix::ones")?;
    let cols = dim_arg(args, 1, "linalg::Matrix::ones")?;
    Ok(matrix_value(&DMatrix::from_element(rows, cols, 1.0)))
}

fn matrix_identity(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::Matrix::identity")?;
    let n = dim_arg(args, 0, "linalg::Matrix::identity")?;
    Ok(matrix_value(&DMatrix::identity(n, n)))
}

fn matrix_diagonal(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::Matrix::diagonal")?;
    let d = as_vector(&args[0], "linalg::Matrix::diagonal")?;
    Ok(matrix_value(&DMatrix::from_diagonal(&d)))
}

fn matrix_from_rows(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::Matrix::from_rows")?;
    let m = as_matrix(&args[0], "linalg::Matrix::from_rows")?;
    Ok(matrix_value(&m))
}

fn matrix_from_cols(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::Matrix::from_cols")?;
    // The input lists columns; build it as rows and transpose (spec appendix B.2).
    let m = as_matrix(&args[0], "linalg::Matrix::from_cols")?;
    Ok(matrix_value(&m.transpose()))
}

// —— matrix operations (spec appendix B.2) ——

fn transpose(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::transpose")?;
    let m = as_matrix(&args[0], "linalg::transpose")?;
    Ok(matrix_value(&m.transpose()))
}

fn inverse(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::inverse")?;
    let m = as_matrix(&args[0], "linalg::inverse")?;
    require_square(&m, "linalg::inverse")?;
    m.clone()
        .try_inverse()
        .map(|inv| matrix_value(&inv))
        .ok_or_else(|| type_err("linalg::inverse", "matrix is singular (R0004)"))
}

fn determinant(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::determinant")?;
    let m = as_matrix(&args[0], "linalg::determinant")?;
    require_square(&m, "linalg::determinant")?;
    Ok(scalar(m.determinant()))
}

fn trace(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::trace")?;
    let m = as_matrix(&args[0], "linalg::trace")?;
    require_square(&m, "linalg::trace")?;
    Ok(scalar(m.trace()))
}

fn rank(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::rank")?;
    let m = as_matrix(&args[0], "linalg::rank")?;
    Ok(scalar(matrix_rank(&m) as f64))
}

fn norm(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    if !(1..=2).contains(&args.len()) {
        return Err(RuntimeError::Message(format!(
            "`linalg::norm` expects 1 or 2 argument(s), got {}",
            args.len()
        )));
    }
    let p = match args.get(1) {
        None => 2.0,
        Some(Value::Number(n)) => n.to_f64_lossy(),
        Some(other) => return Err(type_err("linalg::norm", format!("p must be a number, got {other:?}"))),
    };
    // Nested arrays are matrices; flat numeric arrays are vectors (spec §11.3).
    let is_matrix = matches!(&args[0], Value::Array(rows) if !rows.is_empty() && matches!(&rows[0], Value::Array(_)));
    if is_matrix {
        let m = as_matrix(&args[0], "linalg::norm")?;
        Ok(scalar(matrix_norm(&m, p, "linalg::norm")?))
    } else {
        let v = as_vector(&args[0], "linalg::norm")?;
        Ok(scalar(vector_norm(&v, p, "linalg::norm")?))
    }
}

fn cond(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::cond")?;
    let m = as_matrix(&args[0], "linalg::cond")?;
    let sv = m.clone().svd(false, false).singular_values;
    let max = sv.iter().copied().fold(0.0_f64, f64::max);
    let min = sv.iter().copied().fold(f64::INFINITY, f64::min);
    Ok(scalar(if min > 0.0 { max / min } else { f64::INFINITY }))
}

// —— vector operations (spec appendix B.2) ——

fn dot(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "linalg::dot")?;
    let a = as_vector(&args[0], "linalg::dot")?;
    let b = as_vector(&args[1], "linalg::dot")?;
    if a.len() != b.len() {
        return Err(type_err(
            "linalg::dot",
            format!("dimension mismatch (R0004): {} vs {}", a.len(), b.len()),
        ));
    }
    Ok(scalar(a.dot(&b)))
}

fn cross(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "linalg::cross")?;
    let a = as_vector(&args[0], "linalg::cross")?;
    let b = as_vector(&args[1], "linalg::cross")?;
    if a.len() != 3 || b.len() != 3 {
        return Err(type_err("linalg::cross", "requires two 3-vectors (R0004)"));
    }
    let out = DVector::from_column_slice(&[
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]);
    Ok(vector_value(&out))
}

// —— decompositions (spec appendix B.2) ——

fn lu(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::lu")?;
    let m = as_matrix(&args[0], "linalg::lu")?;
    let lu = m.clone().lu();
    Ok(Value::Tuple(vec![matrix_value(&lu.l()), matrix_value(&lu.u())]))
}

fn qr(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::qr")?;
    let m = as_matrix(&args[0], "linalg::qr")?;
    let qr = m.clone().qr();
    Ok(Value::Tuple(vec![matrix_value(&qr.q()), matrix_value(&qr.r())]))
}

fn svd(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::svd")?;
    let m = as_matrix(&args[0], "linalg::svd")?;
    let svd = m.clone().svd(true, true);
    let u = svd.u.as_ref().ok_or_else(|| type_err("linalg::svd", "failed to compute U"))?;
    let vt = svd.v_t.as_ref().ok_or_else(|| type_err("linalg::svd", "failed to compute Vt"))?;
    Ok(Value::Tuple(vec![
        matrix_value(u),
        vector_value(&svd.singular_values),
        matrix_value(vt),
    ]))
}

fn eigen(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::eigen")?;
    let m = as_matrix(&args[0], "linalg::eigen")?;
    require_square(&m, "linalg::eigen")?;
    let eig = m.clone().symmetric_eigen();
    Ok(Value::Tuple(vec![
        vector_value(&eig.eigenvalues),
        matrix_value(&eig.eigenvectors),
    ]))
}

fn cholesky(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "linalg::cholesky")?;
    let m = as_matrix(&args[0], "linalg::cholesky")?;
    require_square(&m, "linalg::cholesky")?;
    let l = m
        .clone()
        .cholesky()
        .map(|c| c.l())
        .ok_or_else(|| type_err("linalg::cholesky", "matrix is not positive-definite (R0005)"))?;
    Ok(matrix_value(&l))
}

// —— linear solvers (spec appendix B.2) ——

/// Right-hand side of a linear system: either a vector or a matrix.
enum Rhs {
    Vector(DVector<f64>),
    Matrix(DMatrix<f64>),
}

fn as_rhs(v: &Value, fname: &str) -> Result<Rhs, RuntimeError> {
    if matches!(v, Value::Array(rows) if !rows.is_empty() && matches!(&rows[0], Value::Array(_))) {
        as_matrix(v, fname).map(Rhs::Matrix)
    } else {
        as_vector(v, fname).map(Rhs::Vector)
    }
}

/// Materialize the RHS as a matrix (a vector becomes an `n x 1` column).
fn rhs_as_matrix(rhs: &Rhs) -> DMatrix<f64> {
    match rhs {
        Rhs::Vector(v) => {
            let mut m = DMatrix::zeros(v.len(), 1);
            for (i, x) in v.iter().enumerate() {
                m[(i, 0)] = *x;
            }
            m
        }
        Rhs::Matrix(m) => m.clone(),
    }
}

/// Convert the solved `n x k` matrix back to a value, matching the RHS shape.
fn rhs_solution(out: &DMatrix<f64>, rhs: &Rhs) -> Value {
    match rhs {
        Rhs::Vector(_) => Value::Array((0..out.nrows()).map(|r| scalar(out[(r, 0)])).collect()),
        Rhs::Matrix(_) => matrix_value(out),
    }
}

fn solve(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "linalg::solve")?;
    let a = as_matrix(&args[0], "linalg::solve")?;
    let rhs = as_rhs(&args[1], "linalg::solve")?;
    require_square(&a, "linalg::solve")?;
    let b = rhs_as_matrix(&rhs);
    if a.nrows() != b.nrows() {
        return Err(type_err(
            "linalg::solve",
            format!(
                "dimension mismatch (R0004): A is {}x{}, b has {} rows",
                a.nrows(),
                a.ncols(),
                b.nrows()
            ),
        ));
    }
    let x = a
        .clone()
        .lu()
        .solve(&b)
        .ok_or_else(|| type_err("linalg::solve", "singular or unsolvable system (R0004)"))?;
    Ok(rhs_solution(&x, &rhs))
}

fn lstsq(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "linalg::lstsq")?;
    let a = as_matrix(&args[0], "linalg::lstsq")?;
    let rhs = as_rhs(&args[1], "linalg::lstsq")?;
    let b = rhs_as_matrix(&rhs);
    if a.nrows() != b.nrows() {
        return Err(type_err(
            "linalg::lstsq",
            format!(
                "dimension mismatch (R0004): A is {}x{}, b has {} rows",
                a.nrows(),
                a.ncols(),
                b.nrows()
            ),
        ));
    }
    // Least squares via the SVD pseudo-inverse; singular values below a scale-relative
    // tolerance are treated as zero (spec appendix B.2).
    let max_sv = a
        .clone()
        .svd(false, false)
        .singular_values
        .iter()
        .copied()
        .fold(0.0_f64, f64::max);
    let eps = max_sv * (a.nrows().max(a.ncols()) as f64) * f64::EPSILON * 10.0;
    let pinv = a
        .clone()
        .svd(true, true)
        .pseudo_inverse(eps)
        .map_err(|e| RuntimeError::Message(format!("`linalg::lstsq` failed: {e}")))?;
    let x = pinv * b;
    Ok(rhs_solution(&x, &rhs))
}
