use prima_core::Value;
use prima_runtime::Evaluator;

/// Evaluate an in-memory program importing the `stats` namespace (spec §B.3).
fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

/// Extract an f64 from a numeric `Value`.
fn f64_of(v: Value) -> f64 {
    match v {
        Value::Number(n) => n.to_f64_lossy(),
        other => panic!("expected Number, got {other:?}"),
    }
}

fn approx(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "expected {a} ≈ {b} within {tol}",);
}

#[test]
fn mean_of_three() {
    approx(
        f64_of(eval("import stats;\nstats::mean([1.0, 2.0, 3.0])")),
        2.0,
        1e-9,
    );
}

#[test]
fn median_odd_and_even() {
    approx(
        f64_of(eval("import stats;\nstats::median([3.0, 1.0, 2.0])")),
        2.0,
        1e-9,
    );
    approx(
        f64_of(eval("import stats;\nstats::median([1.0, 2.0, 3.0, 4.0])")),
        2.5,
        1e-9,
    );
}

#[test]
fn sample_variance_and_std() {
    approx(
        f64_of(eval("import stats;\nstats::variance([2.0, 4.0, 6.0])")),
        4.0,
        1e-9,
    );
    approx(
        f64_of(eval("import stats;\nstats::std([2.0, 4.0, 6.0])")),
        2.0,
        1e-9,
    );
}

#[test]
fn quantile_interpolates_between_ranks() {
    approx(
        f64_of(eval(
            "import stats;\nstats::quantile([1.0, 2.0, 3.0, 4.0], 0.5)",
        )),
        2.5,
        1e-9,
    );
}

#[test]
fn percentile_matches_quantile() {
    approx(
        f64_of(eval(
            "import stats;\nstats::percentile([1.0, 2.0, 3.0, 4.0], 50)",
        )),
        2.5,
        1e-9,
    );
}

#[test]
fn range_and_extrema() {
    approx(
        f64_of(eval("import stats;\nstats::range([3.0, 1.0, 4.0])")),
        3.0,
        1e-9,
    );
    approx(
        f64_of(eval("import stats;\nstats::min([3.0, 1.0, 4.0])")),
        1.0,
        1e-9,
    );
    approx(
        f64_of(eval("import stats;\nstats::max([3.0, 1.0, 4.0])")),
        4.0,
        1e-9,
    );
}

#[test]
fn mode_most_frequent_and_lowest_on_ties() {
    approx(
        f64_of(eval("import stats;\nstats::mode([1.0, 2.0, 2.0, 3.0])")),
        2.0,
        1e-9,
    );
    approx(
        f64_of(eval("import stats;\nstats::mode([1.0, 1.0, 2.0, 2.0])")),
        1.0,
        1e-9,
    );
}

#[test]
fn covariance_is_sample() {
    approx(
        f64_of(eval(
            "import stats;\nstats::cov([1.0, 2.0, 3.0], [2.0, 4.0, 6.0])",
        )),
        2.0,
        1e-9,
    );
}

#[test]
fn pearson_correlation_of_perfectly_correlated_data() {
    approx(
        f64_of(eval(
            "import stats;\nstats::corr([1.0, 2.0, 3.0], [2.0, 4.0, 6.0])",
        )),
        1.0,
        1e-9,
    );
}

#[test]
fn spearman_correlation_of_monotone_data() {
    approx(
        f64_of(eval(
            "import stats;\nstats::spearman([1.0, 2.0, 3.0], [2.0, 4.0, 6.0])",
        )),
        1.0,
        1e-9,
    );
}

#[test]
fn normal_pdf_at_mean() {
    approx(
        f64_of(eval(
            "import stats;\nstats::pdf(stats::Normal(0.0, 1.0), 0.0)",
        )),
        0.3989423,
        1e-6,
    );
}

#[test]
fn normal_cdf_at_mean() {
    approx(
        f64_of(eval(
            "import stats;\nstats::cdf(stats::Normal(0.0, 1.0), 0.0)",
        )),
        0.5,
        1e-9,
    );
}

#[test]
fn normal_quantile_median() {
    approx(
        f64_of(eval(
            "import stats;\nstats::quantile(stats::Normal(0.0, 1.0), 0.5)",
        )),
        0.0,
        1e-9,
    );
}

#[test]
fn uniform_pdf_cdf_quantile() {
    approx(
        f64_of(eval(
            "import stats;\nstats::pdf(stats::Uniform(0.0, 2.0), 1.0)",
        )),
        0.5,
        1e-9,
    );
    approx(
        f64_of(eval(
            "import stats;\nstats::cdf(stats::Uniform(0.0, 2.0), 1.0)",
        )),
        0.5,
        1e-9,
    );
    approx(
        f64_of(eval(
            "import stats;\nstats::quantile(stats::Uniform(0.0, 2.0), 0.25)",
        )),
        0.5,
        1e-9,
    );
}

#[test]
fn exponential_pdf_cdf() {
    approx(
        f64_of(eval(
            "import stats;\nstats::pdf(stats::Exponential(1.0), 0.0)",
        )),
        1.0,
        1e-9,
    );
    approx(
        f64_of(eval(
            "import stats;\nstats::cdf(stats::Exponential(1.0), 0.0)",
        )),
        0.0,
        1e-9,
    );
    approx(
        f64_of(eval(
            "import stats;\nstats::cdf(stats::Exponential(1.0), 1.0)",
        )),
        1.0 - std::f64::consts::E.powf(-1.0),
        1e-6,
    );
}

#[test]
fn binomial_pmf_and_cdf() {
    // P(X = 2) for Binomial(4, 0.5) = 6/16 = 0.375
    approx(
        f64_of(eval(
            "import stats;\nstats::pdf(stats::Binomial(4, 0.5), 2.0)",
        )),
        0.375,
        1e-6,
    );
    // P(X <= 2) = (1 + 4 + 6)/16 = 11/16 = 0.6875
    approx(
        f64_of(eval(
            "import stats;\nstats::cdf(stats::Binomial(4, 0.5), 2.0)",
        )),
        0.6875,
        1e-6,
    );
    approx(
        f64_of(eval(
            "import stats;\nstats::quantile(stats::Binomial(4, 0.5), 0.5)",
        )),
        2.0,
        1e-9,
    );
}

#[test]
fn poisson_pmf_and_cdf() {
    // P(X = 2) for Poisson(1) = e^-1 / 2
    let v = f64_of(eval("import stats;\nstats::pdf(stats::Poisson(1.0), 2.0)"));
    approx(v, std::f64::consts::E.powf(-1.0) / 2.0, 1e-6);
    // P(X <= 2) = e^-1 (1 + 1 + 1/2)
    let v = f64_of(eval("import stats;\nstats::cdf(stats::Poisson(1.0), 2.0)"));
    approx(v, std::f64::consts::E.powf(-1.0) * 2.5, 1e-6);
}

#[test]
fn sample_uniform_returns_array_in_range() {
    let v = eval("import stats;\nstats::sample(stats::Uniform(0.0, 1.0), 5)");
    match v {
        Value::Array(items) => {
            assert_eq!(items.len(), 5, "expected 5 samples");
            for it in items {
                let x = match it {
                    Value::Number(n) => n.to_f64_lossy(),
                    other => panic!("sample element not numeric: {other:?}"),
                };
                assert!((0.0..1.0).contains(&x), "sample out of range: {x}");
            }
        }
        other => panic!("expected Array, got {other:?}"),
    }
}

#[test]
fn sample_normal_zero_count_is_empty() {
    let v = eval("import stats;\nstats::sample(stats::Normal(0.0, 1.0), 0)");
    assert_eq!(v, Value::Array(vec![]));
}

/// Evaluate an in-memory program expected to error; assert that it does.
fn eval_err(src: &str) {
    prima_stdlib::init();
    assert!(
        Evaluator::new().eval_value(src).is_err(),
        "expected eval to fail for: {src}"
    );
}

#[test]
fn sample_rejects_negative_count() {
    eval_err("import stats;\nstats::sample(stats::Normal(0.0, 1.0), -1)");
}

#[test]
fn mean_of_empty_errors() {
    eval_err("import stats;\nstats::mean([])");
}

#[test]
fn variance_needs_two_points() {
    eval_err("import stats;\nstats::variance([1.0])");
}

#[test]
fn normal_rejects_nonpositive_sigma() {
    eval_err("import stats;\nstats::Normal(0.0, -1.0)");
}
