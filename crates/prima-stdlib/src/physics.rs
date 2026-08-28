//! `physics` module (spec §7.3): CODATA 2022 physical constants.
//!
//! The spec presents the constants as TeX long names (`physics::\planck_const`); the stdlib
//! registry keys are plain identifiers, so this MVP registers them under the bare names
//! (`physics::planck_const`), keeping the same spelling minus the leading backslash.
//! Access is `physics::planck_const` (qualified) or `from physics import planck_const as h`.
//!
//! Constants whose SI definitions are exact are stored as `Number::Rational` (auto-reduced
//! exact division, spec §6.1) or `Number::Integer`; CODATA-measured constants are stored as
//! `Number::Real(Real::F64)` and stay exact-looking only through explicit collapse (§6.1).
//! `standard_gravity` deviates: it is SI-exact but stored as `f64` so it renders as `9.80665`.

use std::collections::HashMap;

use num_bigint::BigInt;
use prima_core::{Number, Real, Value};
use prima_runtime::NamespaceItem;
use prima_runtime::stdlib::register_namespace;

/// Wrap a `Number` as a module-namespace constant value (spec §15.2).
fn val(n: Number) -> NamespaceItem {
    NamespaceItem::Val(Value::Number(n))
}

/// Exact integer from decimal text (sizes exceed `i64` for some constants).
fn exact_int(s: &str) -> Number {
    Number::Integer(BigInt::parse_bytes(s.as_bytes(), 10).expect("valid decimal literal"))
}

/// Exact rational `numer / denom` as decimal text; exact-layer division auto-reduces (spec §6.1).
fn exact_rat(numer: &str, denom: &str) -> Number {
    exact_int(numer) / exact_int(denom)
}

/// Measured (CODATA 2022) constant stored as an inexact `f64` (spec §6.1).
fn real(v: f64) -> Number {
    Number::Real(Real::F64(v))
}

/// Register the `physics` namespace (spec §7.3, CODATA 2022 values).
pub fn register() {
    let mut items = HashMap::new();
    // — base —
    items.insert("speed_of_light".into(), val(exact_int("299792458")));
    items.insert(
        "planck_const".into(),
        val(exact_rat("662607015", "1000000000000000000000000000")),
    );
    items.insert(
        "reduced_planck".into(),
        val(real(6.626_070_15e-34 / (2.0 * std::f64::consts::PI))),
    );
    items.insert(
        "boltzmann_const".into(),
        val(exact_rat("1380649", "100000000000000000000000000000")),
    );
    items.insert("gravitational_const".into(), val(real(6.674_30e-11)));
    // — electromagnetism —
    items.insert(
        "elementary_charge".into(),
        val(exact_rat("1602176634", "1000000000000000000000000000")),
    );
    items.insert("vacuum_permittivity".into(), val(real(8.854_187_812_8e-12)));
    items.insert("vacuum_permeability".into(), val(real(1.256_637_062_12e-6)));
    items.insert("fine_structure".into(), val(real(7.297_352_569_3e-3)));
    // — chemistry —
    items.insert(
        "avogadro_const".into(),
        val(exact_int("602214076000000000000000")),
    );
    items.insert(
        "gas_const".into(),
        val(exact_rat("831446261815324", "100000000000000")),
    );
    items.insert("atomic_mass_unit".into(), val(real(1.660_539_066_60e-27)));
    // — masses —
    items.insert("electron_mass".into(), val(real(9.109_383_701_5e-31)));
    items.insert("proton_mass".into(), val(real(1.672_621_923_69e-27)));
    items.insert("neutron_mass".into(), val(real(1.674_927_498_04e-27)));
    // — quantum —
    items.insert("rydberg".into(), val(real(10_973_731.568_160)));
    items.insert("bohr_radius".into(), val(real(5.291_772_109_03e-11)));
    items.insert("bohr_magneton".into(), val(real(9.274_010_078_3e-24)));
    // — other —
    items.insert("standard_gravity".into(), val(real(9.806_65)));
    items.insert("stefan_boltzmann".into(), val(real(5.670_374_419e-8)));
    items.insert("standard_atmosphere".into(), val(exact_int("101325")));
    register_namespace("physics", items);
}
