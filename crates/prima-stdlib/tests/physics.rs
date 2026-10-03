use prima_core::Value;
use prima_runtime::{Evaluator, RuntimeError};

/// Evaluate an in-memory program that imports the `physics` stdlib module (spec §7.3 / §18.6).
fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

/// Evaluate a program expected to fail with a runtime error.
fn eval_err(src: &str) -> RuntimeError {
    prima_stdlib::init();
    Evaluator::new()
        .eval_value(src)
        .expect_err("expected a runtime error")
}

fn fmt(src: &str) -> String {
    Evaluator::new().format_value(&eval(src))
}

/// The `f64` value of a numeric result.
fn num(src: &str) -> f64 {
    match eval(src) {
        Value::Number(n) => n.to_f64_lossy(),
        other => panic!("expected Number, got {other:?}"),
    }
}

fn close(a: f64, b: f64) {
    assert!(
        (a - b).abs() <= 1e-9 * b.abs().max(1.0),
        "expected {b}, got {a}"
    );
}

// —— constants (spec §7.3) ——

#[test]
fn physics_speed_of_light_exact_integer() {
    assert_eq!(fmt("import physics;\nphysics::speed_of_light"), "299792458");
}

#[test]
fn physics_standard_gravity_f64() {
    assert_eq!(fmt("import physics;\nphysics::standard_gravity"), "9.80665");
}

#[test]
fn physics_planck_times_light_evaluates() {
    let v = eval("import physics;\nphysics::planck_const * physics::speed_of_light");
    match v {
        Value::Number(_) => {}
        other => panic!("expected Number, got {other:?}"),
    }
}

#[test]
fn physics_from_import_alias() {
    let h = eval("from physics import planck_const as h;\nh");
    let direct = eval("import physics;\nphysics::planck_const");
    assert_eq!(h, direct);
}

#[test]
fn physics_avogadro_is_number() {
    match eval("import physics;\nphysics::avogadro_const") {
        Value::Number(_) => {}
        other => panic!("expected Number, got {other:?}"),
    }
}

#[test]
fn physics_reduced_planck_is_measured_f64() {
    close(
        num("import physics;\nphysics::reduced_planck"),
        1.0545718176461565e-34,
    );
}

#[test]
fn physics_exact_constants_stay_exact() {
    // SI-exact constants keep their Integer/Rational forms (spec §6.1) rather than collapsing to f64.
    assert_eq!(
        fmt("import physics;\nphysics::elementary_charge"),
        "\\frac{801088317}{500000000000000000000000000}"
    );
    assert_eq!(
        fmt("import physics;\nphysics::gas_const"),
        "\\frac{207861565453831}{25000000000000}"
    );
    assert_eq!(
        fmt("import physics;\nphysics::standard_atmosphere"),
        "101325"
    );
}

// —— formulas (spec §18.6) ——

#[test]
fn physics_kinematics() {
    close(
        num("import physics;\nphysics::velocity(0.0, 2.0, 3.0)"),
        6.0,
    );
    close(
        num("import physics;\nphysics::displacement(0.0, 2.0, 3.0)"),
        9.0,
    );
    close(
        num("import physics;\nphysics::velocity(1.0, 0.0, 5.0)"),
        1.0,
    );
    // π/4 launch: sin(2θ) = 1, sqrt(2)/2 vertical component.
    close(
        num("import physics;\nphysics::projectile_range(10.0, 0.7853981633974483, 10.0)"),
        10.0,
    );
    close(
        num("import physics;\nphysics::projectile_height(10.0, 0.7853981633974483, 10.0)"),
        2.5,
    );
    close(
        num("import physics;\nphysics::projectile_range(10.0, 0.0, 10.0)"),
        0.0,
    );
}

#[test]
fn physics_mechanics() {
    close(num("import physics;\nphysics::force(2.0, 3.0)"), 6.0);
    close(num("import physics;\nphysics::momentum(2.0, 3.0)"), 6.0);
    close(
        num("import physics;\nphysics::kinetic_energy(2.0, 3.0)"),
        9.0,
    );
    close(
        num("import physics;\nphysics::potential_energy(2.0, 10.0, 3.0)"),
        60.0,
    );
    close(num("import physics;\nphysics::work(2.0, 3.0)"), 6.0);
    close(num("import physics;\nphysics::power(6.0, 3.0)"), 2.0);
}

#[test]
fn physics_simple_harmonic_motion() {
    close(
        num("import physics;\nphysics::shm_displacement(1.0, 1.0, 0.0)"),
        1.0,
    );
    close(
        num("import physics;\nphysics::shm_velocity(1.0, 1.0, 0.0)"),
        0.0,
    );
    close(
        num("import physics;\nphysics::shm_energy(2.0, 3.0, 4.0)"),
        144.0,
    );
    let g = 9.80665;
    let expected = 2.0 * std::f64::consts::PI * (1.0f64 / g).sqrt();
    close(
        num("import physics;\nphysics::simple_pendulum(1.0, 9.80665)"),
        expected,
    );
}

#[test]
fn physics_thermodynamics() {
    close(
        num("import physics;\nphysics::celsius_to_kelvin(0.0)"),
        273.15,
    );
    close(
        num("import physics;\nphysics::kelvin_to_celsius(273.15)"),
        0.0,
    );
    close(num("import physics;\nphysics::heat(1.0, 2.0, 3.0)"), 6.0);
    close(
        num("import physics;\nphysics::ideal_gas_pressure(1.0, 300.0, 1.0)"),
        1.380649e-23 * 300.0,
    );
}

#[test]
fn physics_electromagnetism() {
    // k_e = 1/(4π·ε₀) ≈ 8.9875517923e9.
    close(
        num("import physics;\nphysics::coulomb_force(1.0, 1.0, 1.0)"),
        8.9875517923e9,
    );
    close(num("import physics;\nphysics::ohm_voltage(2.0, 3.0)"), 6.0);
    close(num("import physics;\nphysics::ohm_current(6.0, 3.0)"), 2.0);
    close(
        num("import physics;\nphysics::ohm_resistance(6.0, 2.0)"),
        3.0,
    );
    close(
        num("import physics;\nphysics::electrical_power(2.0, 3.0)"),
        6.0,
    );
}

#[test]
fn physics_formulas_accept_integers() {
    // Integer arguments are accepted (collapsed to F64) just like measured F64 values.
    close(num("import physics;\nphysics::force(2, 3)"), 6.0);
}

#[test]
fn physics_arity_error() {
    let err = eval_err("import physics;\nphysics::force(1.0)");
    assert!(
        err.to_string().contains("expects 2 argument"),
        "unexpected error: {err}"
    );
}

#[test]
fn physics_type_error_on_non_number() {
    let err = eval_err("import physics;\nphysics::force(\"x\", 1.0)");
    assert!(
        err.to_string().contains("must be a number"),
        "unexpected error: {err}"
    );
}

// —— Vector3 (spec §18.6) ——

#[test]
fn physics_vector3_fields_and_arithmetic() {
    close(
        num("import physics;\nlet v = physics::Vector3::new(1.0, 2.0, 3.0);\nv.x"),
        1.0,
    );
    close(
        num("import physics;\nlet v = physics::Vector3::new(1.0, 2.0, 3.0);\nv.z"),
        3.0,
    );
    let src = "import physics;\n\
        let a = physics::Vector3::new(1.0, 2.0, 3.0);\n\
        let b = physics::Vector3::new(4.0, 5.0, 6.0);\n\
        a.add(b).y";
    close(num(src), 7.0);
    let src = "import physics;\n\
        let a = physics::Vector3::new(1.0, 2.0, 3.0);\n\
        let b = physics::Vector3::new(4.0, 5.0, 6.0);\n\
        a.sub(b).z";
    close(num(src), -3.0);
    let src = "import physics;\n\
        let a = physics::Vector3::new(1.0, 2.0, 3.0);\n\
        a.scale(2.0).x";
    close(num(src), 2.0);
}

#[test]
fn physics_vector3_products_and_norm() {
    close(
        num(
            "import physics;\nlet a = physics::Vector3::new(1.0, 0.0, 0.0);\nlet b = physics::Vector3::new(0.0, 1.0, 0.0);\na.dot(b)",
        ),
        0.0,
    );
    close(
        num(
            "import physics;\nlet a = physics::Vector3::new(1.0, 2.0, 3.0);\nlet b = physics::Vector3::new(4.0, 5.0, 6.0);\na.dot(b)",
        ),
        32.0,
    );
    let src = "import physics;\n\
        let a = physics::Vector3::new(1.0, 0.0, 0.0);\n\
        let b = physics::Vector3::new(0.0, 1.0, 0.0);\n\
        a.cross(b).z";
    close(num(src), 1.0);
    close(
        num("import physics;\nphysics::Vector3::new(3.0, 4.0, 0.0).length()"),
        5.0,
    );
    close(
        num(
            "import physics;\nlet v = physics::Vector3::new(0.0, 0.0, 2.0);\nv.normalize().length()",
        ),
        1.0,
    );
}

#[test]
fn physics_vector3_imported_directly() {
    let src = "from physics import Vector3;\nlet v = Vector3::new(3.0, 4.0, 0.0);\nv.length()";
    close(num(src), 5.0);
}

#[test]
fn physics_vector3_normalize_zero_is_unchanged() {
    let src = "import physics;\nlet v = physics::Vector3::new(0.0, 0.0, 0.0);\nv.normalize().x";
    close(num(src), 0.0);
}
