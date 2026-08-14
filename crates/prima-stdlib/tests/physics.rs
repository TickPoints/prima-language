use prima_core::{Number, Value};
use prima_runtime::Evaluator;

/// Evaluate an in-memory program that imports the `physics` stdlib namespace (spec §7.3).
fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn fmt(src: &str) -> String {
    Evaluator::new().format_value(&eval(src))
}

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
