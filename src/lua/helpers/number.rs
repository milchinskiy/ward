#![allow(
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::missing_const_for_fn
)]

use mlua::{Lua, Value, Variadic};

/// Lua Number object
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let number = lua.create_table()?;
    number.set("is_integer", lua.create_function(is_integer_fn)?)?;
    number.set("is_float", lua.create_function(is_float_fn)?)?;
    number.set("is_number", lua.create_function(is_number_fn)?)?;
    number.set("is_nan", lua.create_function(is_nan_fn)?)?;
    number.set("is_infinity", lua.create_function(is_infinity_fn)?)?;
    number.set("is_finite", lua.create_function(is_finite_fn)?)?;
    number.set("round", lua.create_function(round_fn)?)?;
    number.set("ceil", lua.create_function(ceil_fn)?)?;
    number.set("floor", lua.create_function(floor_fn)?)?;
    number.set("abs", lua.create_function(abs_fn)?)?;
    number.set("clamp", lua.create_function(clamp_fn)?)?;
    number.set("sign", lua.create_function(sign_fn)?)?;
    number.set("random", lua.create_function(random_fn)?)?;
    number.set("avg", lua.create_function(avg_fn)?)?;
    number.set("min", lua.create_function(min_fn)?)?;
    number.set("max", lua.create_function(max_fn)?)?;
    number.set("sum", lua.create_function(sum_fn)?)?;

    Ok(number)
}

fn is_integer_fn(_: &Lua, n: f64) -> mlua::Result<bool> {
    Ok(is_integer(n))
}

fn is_float_fn(_: &Lua, n: f64) -> mlua::Result<bool> {
    Ok(is_float(n))
}

fn is_number_fn(_: &Lua, v: Value) -> mlua::Result<bool> {
    Ok(matches!(v, Value::Number(_)))
}

fn is_nan_fn(_: &Lua, v: Value) -> mlua::Result<bool> {
    Ok(matches!(v, Value::Number(n) if n.is_nan()))
}

fn is_infinity_fn(_: &Lua, v: Value) -> mlua::Result<bool> {
    Ok(matches!(v, Value::Number(n) if n.is_infinite()))
}

fn is_finite_fn(_: &Lua, v: Value) -> mlua::Result<bool> {
    Ok(matches!(v, Value::Number(n) if n.is_finite()))
}

fn round_fn(_: &Lua, (n, precision): (f64, i32)) -> mlua::Result<f64> {
    Ok(round(n, precision))
}

fn ceil_fn(_: &Lua, n: f64) -> mlua::Result<f64> {
    Ok(n.ceil())
}

fn floor_fn(_: &Lua, n: f64) -> mlua::Result<f64> {
    Ok(n.floor())
}

fn abs_fn(_: &Lua, n: f64) -> mlua::Result<f64> {
    Ok(n.abs())
}

fn clamp_fn(_: &Lua, (n, min, max): (f64, f64, f64)) -> mlua::Result<f64> {
    Ok(n.clamp(min, max))
}

fn sign_fn(_: &Lua, n: f64) -> mlua::Result<i8> {
    Ok(sign(n))
}

fn random_fn(_: &Lua, (min, max): (f64, f64)) -> mlua::Result<f64> {
    Ok(random(min, max))
}

fn avg_fn(_: &Lua, nums: Variadic<f64>) -> mlua::Result<f64> {
    aggregate(&nums, Aggregate::Average)
}

fn min_fn(_: &Lua, nums: Variadic<f64>) -> mlua::Result<f64> {
    aggregate(&nums, Aggregate::Min)
}

fn max_fn(_: &Lua, nums: Variadic<f64>) -> mlua::Result<f64> {
    aggregate(&nums, Aggregate::Max)
}

fn sum_fn(_: &Lua, nums: Variadic<f64>) -> mlua::Result<f64> {
    aggregate(&nums, Aggregate::Sum)
}

fn is_integer(n: f64) -> bool {
    n.is_finite() && (n.fract().abs() < f64::EPSILON)
}

fn is_float(n: f64) -> bool {
    n.is_finite() && !is_integer(n)
}

fn round(n: f64, precision: i32) -> f64 {
    if precision == 0 {
        return n.round();
    }

    let factor = 10_f64.powi(precision);
    (n * factor).round() / factor
}

fn sign(n: f64) -> i8 {
    if n > 0.0 {
        1
    } else if n < 0.0 {
        -1
    } else {
        0
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn random(min: f64, max: f64) -> f64 {
    use rand::rngs::SmallRng;
    use rand::{Rng, SeedableRng};

    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    let mut rng = SmallRng::from_os_rng();

    if is_integer(min) && is_integer(max) {
        let min_i = min as i64;
        let max_i = max as i64;
        return rng.random_range(min_i..=max_i) as f64;
    }

    rng.random_range(min..=max)
}

enum Aggregate {
    Average,
    Min,
    Max,
    Sum,
}

fn aggregate(values: &Variadic<f64>, mode: Aggregate) -> mlua::Result<f64> {
    if values.is_empty() {
        return Err(mlua::Error::RuntimeError("At least one number is required".to_string()));
    }

    let mut iter = values.iter();
    let mut acc = iter
        .next()
        .copied()
        .ok_or_else(|| mlua::Error::RuntimeError("At least one number is required".to_string()))?;

    #[allow(clippy::cast_precision_loss)]
    let count = values.len() as f64;

    match mode {
        Aggregate::Average => {
            for value in iter {
                acc += value;
            }
            Ok(acc / count)
        }
        Aggregate::Min => {
            for value in iter {
                if value < &acc {
                    acc = *value;
                }
            }
            Ok(acc)
        }
        Aggregate::Max => {
            for value in iter {
                if value > &acc {
                    acc = *value;
                }
            }
            Ok(acc)
        }
        Aggregate::Sum => {
            for value in iter {
                acc += value;
            }
            Ok(acc)
        }
    }
}
