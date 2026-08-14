use prima_core::{Number, Real, Value};
use prima_runtime::Evaluator;

fn f(x: f64) -> Value {
    Value::Number(Number::Real(Real::F64(x)))
}

fn vec(xs: &[f64]) -> Value {
    Value::Array(xs.iter().map(|&x| f(x)).collect())
}

fn mat(rows: &[&[f64]]) -> Value {
    Value::Array(rows.iter().map(|r| vec(r)).collect())
}

/// Evaluate an in-memory program that imports the Rust-hosted `linalg` namespace (spec §18).
fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

/// Assert that evaluating `src` produces a runtime error.
fn eval_err(src: &str) {
    prima_stdlib::init();
    assert!(Evaluator::new().eval_value(src).is_err(), "expected error for: {src}");
}

/// Assert a matrix value is close to the expected rows (elementwise tolerance).
fn assert_mat_close(actual: &Value, expected: &[&[f64]], tol: f64) {
    let Value::Array(rows) = actual else {
        panic!("expected a matrix, got {actual:?}");
    };
    assert_eq!(rows.len(), expected.len(), "row count mismatch: {actual:?}");
    for (row, exp_row) in rows.iter().zip(expected) {
        let Value::Array(cols) = row else {
            panic!("expected a matrix row, got {row:?}");
        };
        assert_eq!(cols.len(), exp_row.len(), "col count mismatch: {actual:?}");
        for (cell, exp) in cols.iter().zip(*exp_row) {
            let Value::Number(n) = cell else {
                panic!("expected a number, got {cell:?}");
            };
            assert!(
                (n.to_f64_lossy() - exp).abs() < tol,
                "expected ~{exp}, got {n}"
            );
        }
    }
}

#[test]
fn matrix_constructors() {
    assert_eq!(eval("import linalg;\nlinalg::Matrix::zeros(2, 2)"), mat(&[&[0.0, 0.0], &[0.0, 0.0]]));
    assert_eq!(eval("import linalg;\nlinalg::Matrix::ones(1, 3)"), mat(&[&[1.0, 1.0, 1.0]]));
    assert_eq!(eval("import linalg;\nlinalg::Matrix::identity(3)"), mat(&[&[1.0, 0.0, 0.0], &[0.0, 1.0, 0.0], &[0.0, 0.0, 1.0]]));
    assert_eq!(eval("import linalg;\nlinalg::Matrix::diagonal([1.0, 2.0])"), mat(&[&[1.0, 0.0], &[0.0, 2.0]]));
}

#[test]
fn matrix_from_rows_and_cols() {
    assert_eq!(
        eval("import linalg;\nlinalg::Matrix::from_rows([[1.0, 2.0], [3.0, 4.0]])"),
        mat(&[&[1.0, 2.0], &[3.0, 4.0]])
    );
    // from_cols treats each inner array as a column (spec appendix B.2).
    assert_eq!(
        eval("import linalg;\nlinalg::Matrix::from_cols([[1.0, 2.0], [3.0, 4.0]])"),
        mat(&[&[1.0, 3.0], &[2.0, 4.0]])
    );
}

#[test]
fn transpose() {
    assert_eq!(
        eval("import linalg;\nlinalg::transpose([[1.0, 2.0], [3.0, 4.0]])"),
        mat(&[&[1.0, 3.0], &[2.0, 4.0]])
    );
}

#[test]
fn determinant_and_trace() {
    assert_eq!(eval("import linalg;\nlinalg::determinant([[1.0, 2.0], [3.0, 4.0]])"), f(-2.0));
    assert_eq!(eval("import linalg;\nlinalg::determinant([[2.0, 0.0], [0.0, 4.0]])"), f(8.0));
    assert_eq!(eval("import linalg;\nlinalg::trace(linalg::Matrix::identity(3))"), f(3.0));
    assert_eq!(eval("import linalg;\nlinalg::trace([[1.0, 2.0], [3.0, 4.0]])"), f(5.0));
}

#[test]
fn inverse() {
    assert_eq!(
        eval("import linalg;\nlinalg::inverse([[1.0, 2.0], [3.0, 4.0]])"),
        mat(&[&[-2.0, 1.0], &[1.5, -0.5]])
    );
    assert_eq!(
        eval("import linalg;\nlinalg::inverse([[2.0, 0.0], [0.0, 4.0]])"),
        mat(&[&[0.5, 0.0], &[0.0, 0.25]])
    );
    eval_err("import linalg;\nlinalg::inverse([[1.0, 2.0], [2.0, 4.0]])");
    eval_err("import linalg;\nlinalg::inverse([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]])");
}

#[test]
fn rank_and_cond() {
    assert_eq!(eval("import linalg;\nlinalg::rank([[1.0, 2.0], [3.0, 4.0]])"), f(2.0));
    assert_eq!(eval("import linalg;\nlinalg::rank([[1.0, 2.0], [2.0, 4.0]])"), f(1.0));
    assert_eq!(eval("import linalg;\nlinalg::rank([[0.0, 0.0], [0.0, 0.0]])"), f(0.0));
    assert_eq!(eval("import linalg;\nlinalg::cond([[1.0, 0.0], [0.0, 2.0]])"), f(2.0));
}

#[test]
fn norm_matrix_and_vector() {
    assert_eq!(eval("import linalg;\nlinalg::norm([3.0, 4.0])"), f(5.0));
    assert_eq!(eval("import linalg;\nlinalg::norm([3.0, 4.0], 1)"), f(7.0));
    assert_eq!(eval("import linalg;\nlinalg::norm([[3.0, 0.0], [0.0, 4.0]])"), f(5.0));
    assert_eq!(eval("import linalg;\nlinalg::norm([[3.0, 0.0], [0.0, 4.0]], 1)"), f(4.0));
    assert_eq!(eval("import linalg;\nlinalg::norm([[3.0, 0.0], [0.0, 4.0]], 1e999)"), f(4.0));
}

#[test]
fn dot_product() {
    assert_eq!(eval("import linalg;\nlinalg::dot([1.0, 2.0], [3.0, 4.0])"), f(11.0));
    assert_eq!(eval("import linalg;\nlinalg::dot([1.0, 0.0], [0.0, 1.0])"), f(0.0));
    eval_err("import linalg;\nlinalg::dot([1.0, 2.0], [1.0, 2.0, 3.0])");
}

#[test]
fn cross_product() {
    assert_eq!(
        eval("import linalg;\nlinalg::cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0])"),
        vec(&[0.0, 0.0, 1.0])
    );
    eval_err("import linalg;\nlinalg::cross([1.0, 0.0], [0.0, 1.0])");
}

#[test]
fn lu_decomposition() {
    let v = eval("import linalg;\nlinalg::lu([[2.0, 1.0], [1.0, 3.0]])");
    let Value::Tuple(items) = v else {
        panic!("expected a Tuple, got {v:?}");
    };
    assert_eq!(items.len(), 2);
    assert_mat_close(&items[0], &[&[1.0, 0.0], &[0.5, 1.0]], 1e-9);
    assert_mat_close(&items[1], &[&[2.0, 1.0], &[0.0, 2.5]], 1e-9);
}

#[test]
fn qr_decomposition_reconstructs() {
    let v = eval("import linalg;\nlinalg::qr([[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]])");
    let Value::Tuple(items) = v else {
        panic!("expected a Tuple, got {v:?}");
    };
    assert_eq!(items.len(), 2);
    let (Value::Array(qrows), Value::Array(rrows)) = (&items[0], &items[1]) else {
        panic!("expected Q and R matrices");
    };
    // Reconstruct Q * R and compare to the input.
    let q: Vec<Vec<f64>> = qrows
        .iter()
        .map(|r| {
            let Value::Array(c) = r else { panic!("expected Q row") };
            c.iter().map(|x| number(x)).collect()
        })
        .collect();
    let r: Vec<Vec<f64>> = rrows
        .iter()
        .map(|r| {
            let Value::Array(c) = r else { panic!("expected R row") };
            c.iter().map(|x| number(x)).collect()
        })
        .collect();
    for i in 0..3 {
        for j in 0..2 {
            let mut s = 0.0;
            for k in 0..2 {
                s += q[i][k] * r[k][j];
            }
            let expected = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0][i * 2 + j];
            assert!((s - expected).abs() < 1e-9, "Q*R[{i},{j}] = {s}, expected {expected}");
        }
    }
}

#[test]
fn svd_decomposition() {
    let v = eval("import linalg;\nlinalg::svd([[1.0, 0.0], [0.0, 2.0]])");
    let Value::Tuple(items) = v else {
        panic!("expected a Tuple, got {v:?}");
    };
    assert_eq!(items.len(), 3);
    // Singular values are sorted in descending order (flat vector).
    assert_eq!(items[1], vec(&[2.0, 1.0]));
    // U * diag(S) * Vt reconstructs the input.
    let Value::Array(rows) = &items[0] else { panic!("expected U") };
    let u: Vec<Vec<f64>> = rows
        .iter()
        .map(|r| {
            let Value::Array(c) = r else { panic!("expected U row") };
            c.iter().map(|x| number(x)).collect()
        })
        .collect();
    let Value::Array(vtrows) = &items[2] else { panic!("expected Vt") };
    let vt: Vec<Vec<f64>> = vtrows
        .iter()
        .map(|r| {
            let Value::Array(c) = r else { panic!("expected Vt row") };
            c.iter().map(|x| number(x)).collect()
        })
        .collect();
    for i in 0..2 {
        for j in 0..2 {
            let mut s = 0.0;
            for k in 0..2 {
                s += u[i][k] * [2.0, 1.0][k] * vt[k][j];
            }
            let expected = [1.0, 0.0, 0.0, 2.0][i * 2 + j];
            assert!((s - expected).abs() < 1e-9, "U*S*Vt[{i},{j}] = {s}, expected {expected}");
        }
    }
}

#[test]
fn eigen_decomposition() {
    let v = eval("import linalg;\nlinalg::eigen([[2.0, 0.0], [0.0, 3.0]])");
    let Value::Tuple(items) = v else {
        panic!("expected a Tuple, got {v:?}");
    };
    assert_eq!(items.len(), 2);
    // Eigenvalues are unsorted; compare after sorting (spec appendix B.2).
    let Value::Array(values) = &items[0] else {
        panic!("expected an eigenvalues vector");
    };
    let mut vals: Vec<f64> = values.iter().map(|x| number(x)).collect();
    vals.sort_by(f64::total_cmp);
    assert!((vals[0] - 2.0).abs() < 1e-9 && (vals[1] - 3.0).abs() < 1e-9, "eigenvalues: {vals:?}");
    eval_err("import linalg;\nlinalg::eigen([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]])");
}

#[test]
fn cholesky_decomposition() {
    assert_eq!(
        eval("import linalg;\nlinalg::cholesky([[4.0, 0.0], [0.0, 9.0]])"),
        mat(&[&[2.0, 0.0], &[0.0, 3.0]])
    );
    eval_err("import linalg;\nlinalg::cholesky([[1.0, 2.0], [2.0, 1.0]])");
    eval_err("import linalg;\nlinalg::cholesky([[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]])");
}

#[test]
fn solve_linear_system() {
    assert_eq!(
        eval("import linalg;\nlinalg::solve([[2.0, 0.0], [0.0, 4.0]], [2.0, 8.0])"),
        vec(&[1.0, 2.0])
    );
    // Matrix RHS: A * X = B with B a 2x2 identity.
    assert_eq!(
        eval("import linalg;\nlinalg::solve([[2.0, 0.0], [0.0, 4.0]], [[1.0, 0.0], [0.0, 1.0]])"),
        mat(&[&[0.5, 0.0], &[0.0, 0.25]])
    );
    eval_err("import linalg;\nlinalg::solve([[1.0, 2.0], [2.0, 4.0]], [1.0, 1.0])");
    eval_err("import linalg;\nlinalg::solve([[1.0, 0.0], [0.0, 1.0]], [1.0, 2.0, 3.0])");
}

#[test]
fn lstsq_overdetermined() {
    // Consistent overdetermined system: [1, 2, 3]^T x0 = [1, 2, 3] → x0 = 1.
    let v = eval("import linalg;\nlinalg::lstsq([[1.0], [2.0], [3.0]], [1.0, 2.0, 3.0])");
    let Value::Array(xs) = &v else {
        panic!("expected a vector, got {v:?}");
    };
    assert_eq!(xs.len(), 1);
    assert!((number(&xs[0]) - 1.0).abs() < 1e-9, "lstsq result: {v:?}");
    eval_err("import linalg;\nlinalg::lstsq([[1.0, 0.0], [0.0, 1.0]], [1.0, 2.0, 3.0])");
}

#[test]
fn dim_mismatch_and_shape_errors() {
    eval_err("import linalg;\nlinalg::determinant([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]])");
    eval_err("import linalg;\nlinalg::trace([[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]])");
    eval_err("import linalg;\nlinalg::determinant([[1.0, 2.0], [3.0]])");
    eval_err("import linalg;\nlinalg::determinant([[\"a\", \"b\"], [\"c\", \"d\"]])");
    eval_err("import linalg;\nlinalg::determinant(42)");
    eval_err("import linalg;\nlinalg::solve([[1.0, 0.0]], [1.0, 0.0, 0.0])");
    eval_err("import linalg;\nlinalg::norm([1.0, 2.0], 0.5)");
    eval_err("import linalg;\nlinalg::Matrix::identity(0)");
}

fn number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.to_f64_lossy(),
        other => panic!("expected a number, got {other:?}"),
    }
}
