//! `physics` module (spec §7.3 / §18.6): CODATA 2022 physical constants, elementary formulas,
//! and the `Vector3` class.
//!
//! The constants and `Vector3` live in the embedded `physics.pra` signature module (spec §18.4);
//! this file registers only the Rust `@builtin` implementations of the formula set (spec §18.6):
//! kinematics, mechanics, simple harmonic motion, thermodynamics, and electromagnetism. Every
//! formula takes and returns `F64` and reports wrong arity or non-real arguments as a
//! [`RuntimeError`].

use prima_core::{Number, Real, Value};
use prima_runtime::builtin;
use prima_runtime::{Evaluator, RuntimeError};

/// Coulomb constant `k_e = 1 / (4π·ε₀)` (N·m²/C²), derived from the CODATA `vacuum_permittivity`.
const COULOMB_CONST: f64 = 1.0 / (4.0 * std::f64::consts::PI * 8.854_187_812_8e-12);

/// Boltzmann constant `k_B` (J/K) as `f64`, for the ideal-gas relation.
const BOLTZMANN_CONST: f64 = 1.380_649e-23;

/// Register the `physics` `@builtin` implementations (spec §18.4 / §18.6). Each `@builtin`
/// declaration in the embedded `physics.pra` module binds to the implementation registered under
/// its fully-qualified `physics::<name>` key.
pub fn register() {
    builtin!("physics::velocity", velocity);
    builtin!("physics::displacement", displacement);
    builtin!("physics::projectile_range", projectile_range);
    builtin!("physics::projectile_height", projectile_height);
    builtin!("physics::force", force);
    builtin!("physics::momentum", momentum);
    builtin!("physics::kinetic_energy", kinetic_energy);
    builtin!("physics::potential_energy", potential_energy);
    builtin!("physics::work", work);
    builtin!("physics::power", power);
    builtin!("physics::shm_displacement", shm_displacement);
    builtin!("physics::shm_velocity", shm_velocity);
    builtin!("physics::shm_energy", shm_energy);
    builtin!("physics::simple_pendulum", simple_pendulum);
    builtin!("physics::celsius_to_kelvin", celsius_to_kelvin);
    builtin!("physics::kelvin_to_celsius", kelvin_to_celsius);
    builtin!("physics::heat", heat);
    builtin!("physics::ideal_gas_pressure", ideal_gas_pressure);
    builtin!("physics::coulomb_force", coulomb_force);
    builtin!("physics::ohm_voltage", ohm_voltage);
    builtin!("physics::ohm_current", ohm_current);
    builtin!("physics::ohm_resistance", ohm_resistance);
    builtin!("physics::electrical_power", electrical_power);
}

// —— argument helpers (mirroring the `math`/`stats` module conventions) ——

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

/// Extract argument `i` as an `f64`; complex numbers are rejected rather than silently truncated.
fn real_arg(args: &[Value], i: usize, fname: &str) -> Result<f64, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) if !n.is_complex() => Ok(n.to_f64_lossy()),
        Some(Value::Number(n)) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a real number, got {n}"
        ))),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a number, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!(
            "`{fname}` missing argument {i}"
        ))),
    }
}

fn args1(args: &[Value], fname: &str) -> Result<f64, RuntimeError> {
    arity(args, 1, fname)?;
    real_arg(args, 0, fname)
}

fn args2(args: &[Value], fname: &str) -> Result<(f64, f64), RuntimeError> {
    arity(args, 2, fname)?;
    Ok((real_arg(args, 0, fname)?, real_arg(args, 1, fname)?))
}

fn args3(args: &[Value], fname: &str) -> Result<(f64, f64, f64), RuntimeError> {
    arity(args, 3, fname)?;
    Ok((
        real_arg(args, 0, fname)?,
        real_arg(args, 1, fname)?,
        real_arg(args, 2, fname)?,
    ))
}

fn f64_value(x: f64) -> Value {
    Value::Number(Number::Real(Real::F64(x)))
}

// —— kinematics (spec §18.6) ——

fn velocity(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (u, a, t) = args3(args, "physics::velocity")?;
    Ok(f64_value(u + a * t))
}

fn displacement(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (u, a, t) = args3(args, "physics::displacement")?;
    Ok(f64_value(u * t + a * t * t / 2.0))
}

fn projectile_range(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (v0, theta, g) = args3(args, "physics::projectile_range")?;
    Ok(f64_value(v0 * v0 * (2.0 * theta).sin() / g))
}

fn projectile_height(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (v0, theta, g) = args3(args, "physics::projectile_height")?;
    let vy = v0 * theta.sin();
    Ok(f64_value(vy * vy / (2.0 * g)))
}

// —— mechanics (spec §18.6) ——

fn force(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (m, a) = args2(args, "physics::force")?;
    Ok(f64_value(m * a))
}

fn momentum(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (m, v) = args2(args, "physics::momentum")?;
    Ok(f64_value(m * v))
}

fn kinetic_energy(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (m, v) = args2(args, "physics::kinetic_energy")?;
    Ok(f64_value(m * v * v / 2.0))
}

fn potential_energy(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (m, g, h) = args3(args, "physics::potential_energy")?;
    Ok(f64_value(m * g * h))
}

fn work(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (f, d) = args2(args, "physics::work")?;
    Ok(f64_value(f * d))
}

fn power(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (w, t) = args2(args, "physics::power")?;
    Ok(f64_value(w / t))
}

// —— simple harmonic motion (spec §18.6) ——

fn shm_displacement(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (amplitude, omega, t) = args3(args, "physics::shm_displacement")?;
    Ok(f64_value(amplitude * (omega * t).cos()))
}

fn shm_velocity(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (amplitude, omega, t) = args3(args, "physics::shm_velocity")?;
    Ok(f64_value(-amplitude * omega * (omega * t).sin()))
}

fn shm_energy(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (m, omega, amplitude) = args3(args, "physics::shm_energy")?;
    Ok(f64_value(m * omega * omega * amplitude * amplitude / 2.0))
}

/// Small-angle period of a simple pendulum; the argument order matches the spec example
/// `physics::simple_pendulum(L, g)`.
fn simple_pendulum(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (length, g) = args2(args, "physics::simple_pendulum")?;
    Ok(f64_value(2.0 * std::f64::consts::PI * (length / g).sqrt()))
}

// —— thermodynamics (spec §18.6) ——

fn celsius_to_kelvin(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let c = args1(args, "physics::celsius_to_kelvin")?;
    Ok(f64_value(c + 273.15))
}

fn kelvin_to_celsius(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let k = args1(args, "physics::kelvin_to_celsius")?;
    Ok(f64_value(k - 273.15))
}

fn heat(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (m, c, delta_t) = args3(args, "physics::heat")?;
    Ok(f64_value(m * c * delta_t))
}

fn ideal_gas_pressure(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (n, temperature, volume) = args3(args, "physics::ideal_gas_pressure")?;
    Ok(f64_value(n * BOLTZMANN_CONST * temperature / volume))
}

// —— electromagnetism (spec §18.6) ——

fn coulomb_force(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (q1, q2, r) = args3(args, "physics::coulomb_force")?;
    Ok(f64_value(COULOMB_CONST * q1 * q2 / (r * r)))
}

fn ohm_voltage(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (i, r) = args2(args, "physics::ohm_voltage")?;
    Ok(f64_value(i * r))
}

fn ohm_current(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (v, r) = args2(args, "physics::ohm_current")?;
    Ok(f64_value(v / r))
}

fn ohm_resistance(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (v, i) = args2(args, "physics::ohm_resistance")?;
    Ok(f64_value(v / i))
}

fn electrical_power(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let (v, i) = args2(args, "physics::electrical_power")?;
    Ok(f64_value(v * i))
}
