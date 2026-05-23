use std::f64::consts::PI;

use reincarnate_core::ir::builder::FunctionBuilder;
use reincarnate_core::ir::func::{InlineHint, Visibility};
use reincarnate_core::ir::inst::CmpKind;
use reincarnate_core::ir::module::Module;
use reincarnate_core::ir::ty::{FunctionSig, Type};

/// Attach IR bodies to GML runtime stubs that have closed-form math definitions.
///
/// Called after `register_runtime` has already created empty stubs for these
/// functions.  We build a full `Function` with `FunctionBuilder`, then replace
/// the stub's `blocks`, `insts`, `value_types`, and `entry` with the built
/// function — the stub's `name`, `sig`, `visibility`, and registry entry are
/// left untouched.
///
/// The bodies use only `*_f64` builtin calls and `Const(Float)` values, so they
/// are legal IR for any pipeline stage that runs after registration.
pub fn register_runtime_bodies(module: &mut Module) {
    attach_body_point_in_rectangle(module);
    attach_body_point_in_circle(module);
    attach_body_lengthdir_x(module);
    attach_body_lengthdir_y(module);
    attach_body_point_distance(module);
    attach_body_degtorad(module);
    attach_body_radtodeg(module);
    attach_body_dsin(module);
    attach_body_dcos(module);
    attach_body_dtan(module);
    attach_body_darcsin(module);
    attach_body_darccos(module);
    attach_body_darctan(module);
    attach_body_darctan2(module);
    attach_body_arctan2(module);
    attach_body_point_direction(module);
    attach_body_sqr(module);
    attach_body_power(module);
    attach_body_logn(module);
    attach_body_log2(module);
    attach_body_log10(module);
    attach_body_exp(module);
    attach_body_clamp(module);
    attach_body_lerp(module);
    attach_body_abs(module);
    attach_body_floor(module);
    attach_body_ceil(module);
    attach_body_round(module);
    attach_body_sign(module);
    attach_body_sqrt(module);
    attach_body_arctan(module);
    attach_body_frac(module);
    attach_body_dot_product(module);
    attach_body_dot_product_3d(module);
    attach_body_color_get_red(module);
    module.register_alias(
        "colour_get_red",
        module.lookup_runtime("color_get_red").unwrap(),
    );
    attach_body_color_get_green(module);
    module.register_alias(
        "colour_get_green",
        module.lookup_runtime("color_get_green").unwrap(),
    );
    attach_body_color_get_blue(module);
    module.register_alias(
        "colour_get_blue",
        module.lookup_runtime("color_get_blue").unwrap(),
    );
    attach_body_make_color_rgb(module);
    module.register_alias(
        "make_colour_rgb",
        module.lookup_runtime("make_color_rgb").unwrap(),
    );
    attach_body_merge_color(module);
    module.register_alias(
        "merge_colour",
        module.lookup_runtime("merge_color").unwrap(),
    );
    attach_body_color_get_value(module);
    module.register_alias(
        "colour_get_value",
        module.lookup_runtime("color_get_value").unwrap(),
    );
    attach_body_color_get_saturation(module);
    module.register_alias(
        "colour_get_saturation",
        module.lookup_runtime("color_get_saturation").unwrap(),
    );
    attach_body_color_get_hue(module);
    module.register_alias(
        "colour_get_hue",
        module.lookup_runtime("color_get_hue").unwrap(),
    );
    attach_body_make_color_hsv(module);
    module.register_alias(
        "make_colour_hsv",
        module.lookup_runtime("make_color_hsv").unwrap(),
    );
    attach_body_string_length(module);
    attach_body_string_upper(module);
    attach_body_string_lower(module);
    attach_body_string_char_at(module);
    attach_body_string_copy(module);
    attach_body_string_pos(module);
    attach_body_string_delete(module);
    attach_body_string_insert(module);
    attach_body_string_replace_all(module);
    attach_body_string_count(module);
    attach_body_string_ord_at(module);
    attach_body_string_repeat(module);
    attach_body_string_replace(module);
    attach_body_string_hash_to_newline(module);
    attach_body_string_trim(module);
    attach_body_array_length(module);
    attach_body_array_length_1d(module);
    attach_body_array_contains(module);
    attach_body_sin(module);
    attach_body_cos(module);
    attach_body_tan(module);
    attach_body_arcsin(module);
    attach_body_arccos(module);
    attach_body_ord(module);
    attach_body_string_byte_at(module);
    attach_body_chr(module);
    attach_body_ln(module);
    attach_body_math_get_epsilon(module);
    attach_body_is_bool(module);
    attach_body_is_real(module);
    attach_body_is_string(module);
    attach_body_is_undefined(module);
    attach_body_is_array(module);
    attach_body_is_method(module);
    attach_body_is_struct(module);
    attach_body_real(module);
}

// ---------------------------------------------------------------------------
// Helper: build a FunctionBuilder pre-loaded with the module's runtime registry
// so that arithmetic helpers (add, mul, etc.) and call_named work correctly.
// ---------------------------------------------------------------------------

fn make_builder(module: &Module, name: &str, sig: FunctionSig) -> FunctionBuilder {
    let registry = module.runtime_registry.clone();
    let mut b = FunctionBuilder::new(name, sig, Visibility::Public);
    b.set_registry(registry);
    b
}

/// Look up `name` in the runtime registry (panicking if not found), build a
/// `FunctionBuilder` with the given signature, call `build` to populate it,
/// then copy the resulting `blocks`, `insts`, `value_types`, and `entry` into
/// the stub in-place and set `inline_hint = InlineHint::Always`.
///
/// The stub's `name`, `sig`, `visibility`, and registry entry are left
/// untouched.
///
/// # Panics
/// Panics if `name` is not in the runtime registry — this is a programming
/// error (the stub must be registered before its body is attached).
fn attach_runtime_body<F>(
    module: &mut Module,
    name: &str,
    params: &[Type],
    return_ty: Type,
    build: F,
) where
    F: FnOnce(&mut FunctionBuilder),
{
    let fid = module
        .lookup_runtime(name)
        .unwrap_or_else(|| panic!("attach_runtime_body: '{name}' not in runtime registry"));
    let sig = FunctionSig {
        params: params.to_vec(),
        return_ty,
        defaults: vec![],
        has_rest_param: false,
        param_lower_bounds: vec![],
    };
    let mut b = make_builder(module, name, sig);
    build(&mut b);
    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
    stub.inline_hint = InlineHint::Always;
}

// ---------------------------------------------------------------------------
// point_in_rectangle(px, py, x1, y1, x2, y2: f64) -> Bool
//   =  px >= x1 && px <= x2 && py >= y1 && py <= y2
// ---------------------------------------------------------------------------

fn attach_body_point_in_rectangle(module: &mut Module) {
    attach_runtime_body(
        module,
        "point_in_rectangle",
        &[
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
        ],
        Type::Bool,
        |b| {
            let px = b.param(0);
            let py = b.param(1);
            let x1 = b.param(2);
            let y1 = b.param(3);
            let x2 = b.param(4);
            let y2 = b.param(5);

            let px_ge_x1 = b.cmp(CmpKind::Ge, px, x1);
            let px_le_x2 = b.cmp(CmpKind::Le, px, x2);
            let py_ge_y1 = b.cmp(CmpKind::Ge, py, y1);
            let py_le_y2 = b.cmp(CmpKind::Le, py, y2);
            let in_x = b.call_named("and_bool", &[px_ge_x1, px_le_x2], Type::Bool);
            let in_y = b.call_named("and_bool", &[py_ge_y1, py_le_y2], Type::Bool);
            let result = b.call_named("and_bool", &[in_x, in_y], Type::Bool);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// point_in_circle(px, py, cx, cy, radius: f64) -> Bool
//   =  (px - cx)^2 + (py - cy)^2 <= radius^2
// ---------------------------------------------------------------------------

fn attach_body_point_in_circle(module: &mut Module) {
    attach_runtime_body(
        module,
        "point_in_circle",
        &[
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
        ],
        Type::Bool,
        |b| {
            let px = b.param(0);
            let py = b.param(1);
            let cx = b.param(2);
            let cy = b.param(3);
            let radius = b.param(4);

            let dx = b.sub(px, cx);
            let dy = b.sub(py, cy);
            let dx2 = b.mul(dx, dx);
            let dy2 = b.mul(dy, dy);
            let dist2 = b.add(dx2, dy2);
            let r2 = b.mul(radius, radius);
            let result = b.cmp(CmpKind::Le, dist2, r2);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// lengthdir_x(len: f64, dir: f64) -> f64  =  len * cos(dir * π/180)
// ---------------------------------------------------------------------------

fn attach_body_lengthdir_x(module: &mut Module) {
    attach_runtime_body(
        module,
        "lengthdir_x",
        &[Type::Float(64), Type::Float(64)],
        Type::Float(64),
        |b| {
            let len = b.param(0);
            let dir = b.param(1);

            let pi_over_180 = b.const_float(PI / 180.0);
            let dir_rad = b.mul(dir, pi_over_180);
            let cos_val = b.call_named("cos_f64", &[dir_rad], Type::Float(64));
            let result = b.mul(len, cos_val);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// lengthdir_y(len: f64, dir: f64) -> f64  =  len * -sin(dir * π/180)
//
// GML uses a y-down coordinate system where angle 0 points right and
// increases counter-clockwise.  The GameMaker manual defines
// `lengthdir_y` as `len * -sin(dir * π/180)` because increasing y goes
// down, which flips the vertical component.
// ---------------------------------------------------------------------------

fn attach_body_lengthdir_y(module: &mut Module) {
    attach_runtime_body(
        module,
        "lengthdir_y",
        &[Type::Float(64), Type::Float(64)],
        Type::Float(64),
        |b| {
            let len = b.param(0);
            let dir = b.param(1);

            let pi_over_180 = b.const_float(PI / 180.0);
            let dir_rad = b.mul(dir, pi_over_180);
            let sin_val = b.call_named("sin_f64", &[dir_rad], Type::Float(64));
            let neg_sin = b.neg(sin_val);
            let result = b.mul(len, neg_sin);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// point_distance(x1, y1, x2, y2: f64) -> f64  =  hypot(x2-x1, y2-y1)
// ---------------------------------------------------------------------------

fn attach_body_point_distance(module: &mut Module) {
    attach_runtime_body(
        module,
        "point_distance",
        &[
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
        ],
        Type::Float(64),
        |b| {
            let x1 = b.param(0);
            let y1 = b.param(1);
            let x2 = b.param(2);
            let y2 = b.param(3);

            let dx = b.sub(x2, x1);
            let dy = b.sub(y2, y1);
            let result = b.call_named("hypot_f64", &[dx, dy], Type::Float(64));
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// degtorad(x: f64) -> f64  =  x * π/180
// ---------------------------------------------------------------------------

fn attach_body_degtorad(module: &mut Module) {
    attach_runtime_body(
        module,
        "degtorad",
        &[Type::Float(64)],
        Type::Float(64),
        |b| {
            let x = b.param(0);
            let factor = b.const_float(PI / 180.0);
            let result = b.mul(x, factor);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// radtodeg(x: f64) -> f64  =  x * 180/π
// ---------------------------------------------------------------------------

fn attach_body_radtodeg(module: &mut Module) {
    attach_runtime_body(
        module,
        "radtodeg",
        &[Type::Float(64)],
        Type::Float(64),
        |b| {
            let x = b.param(0);
            let factor = b.const_float(180.0 / PI);
            let result = b.mul(x, factor);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// dsin(x: f64) -> f64  =  sin(x * π/180)
// ---------------------------------------------------------------------------

fn attach_body_dsin(module: &mut Module) {
    attach_runtime_body(module, "dsin", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let factor = b.const_float(PI / 180.0);
        let rad = b.mul(x, factor);
        let result = b.call_named("sin_f64", &[rad], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// dcos(x: f64) -> f64  =  cos(x * π/180)
// ---------------------------------------------------------------------------

fn attach_body_dcos(module: &mut Module) {
    attach_runtime_body(module, "dcos", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let factor = b.const_float(PI / 180.0);
        let rad = b.mul(x, factor);
        let result = b.call_named("cos_f64", &[rad], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// dtan(x: f64) -> f64  =  tan(x * π/180)
// ---------------------------------------------------------------------------

fn attach_body_dtan(module: &mut Module) {
    attach_runtime_body(module, "dtan", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let factor = b.const_float(PI / 180.0);
        let rad = b.mul(x, factor);
        let result = b.call_named("tan_f64", &[rad], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// darcsin(x: f64) -> f64  =  asin(x) * 180/π
// ---------------------------------------------------------------------------

fn attach_body_darcsin(module: &mut Module) {
    attach_runtime_body(
        module,
        "darcsin",
        &[Type::Float(64)],
        Type::Float(64),
        |b| {
            let x = b.param(0);
            let asin_val = b.call_named("asin_f64", &[x], Type::Float(64));
            let factor = b.const_float(180.0 / PI);
            let result = b.mul(asin_val, factor);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// darccos(x: f64) -> f64  =  acos(x) * 180/π
// ---------------------------------------------------------------------------

fn attach_body_darccos(module: &mut Module) {
    attach_runtime_body(
        module,
        "darccos",
        &[Type::Float(64)],
        Type::Float(64),
        |b| {
            let x = b.param(0);
            let acos_val = b.call_named("acos_f64", &[x], Type::Float(64));
            let factor = b.const_float(180.0 / PI);
            let result = b.mul(acos_val, factor);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// darctan(x: f64) -> f64  =  atan(x) * 180/π
// ---------------------------------------------------------------------------

fn attach_body_darctan(module: &mut Module) {
    attach_runtime_body(
        module,
        "darctan",
        &[Type::Float(64)],
        Type::Float(64),
        |b| {
            let x = b.param(0);
            let atan_val = b.call_named("atan_f64", &[x], Type::Float(64));
            let factor = b.const_float(180.0 / PI);
            let result = b.mul(atan_val, factor);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// darctan2(y: f64, x: f64) -> f64  =  atan2(y, x) * 180/π
// ---------------------------------------------------------------------------

fn attach_body_darctan2(module: &mut Module) {
    attach_runtime_body(
        module,
        "darctan2",
        &[Type::Float(64), Type::Float(64)],
        Type::Float(64),
        |b| {
            let y = b.param(0);
            let x = b.param(1);
            let atan2_val = b.call_named("atan2_f64", &[y, x], Type::Float(64));
            let factor = b.const_float(180.0 / PI);
            let result = b.mul(atan2_val, factor);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// arctan2(y: f64, x: f64) -> f64  =  atan2(y, x)  [result in radians]
// ---------------------------------------------------------------------------

fn attach_body_arctan2(module: &mut Module) {
    attach_runtime_body(
        module,
        "arctan2",
        &[Type::Float(64), Type::Float(64)],
        Type::Float(64),
        |b| {
            let y = b.param(0);
            let x = b.param(1);
            let result = b.call_named("atan2_f64", &[y, x], Type::Float(64));
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// point_direction(x1, y1, x2, y2: f64) -> f64
//   = atan2(y1 - y2, x2 - x1) * 180/π
//
// GML uses a y-down coordinate system, so the angle from (x1,y1) toward
// (x2,y2) is computed as atan2(y1-y2, x2-x1) — the y delta is inverted
// relative to standard math convention.
// ---------------------------------------------------------------------------

fn attach_body_point_direction(module: &mut Module) {
    attach_runtime_body(
        module,
        "point_direction",
        &[
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
        ],
        Type::Float(64),
        |b| {
            let x1 = b.param(0);
            let y1 = b.param(1);
            let x2 = b.param(2);
            let y2 = b.param(3);

            let dy = b.sub(y1, y2);
            let dx = b.sub(x2, x1);
            let atan2_val = b.call_named("atan2_f64", &[dy, dx], Type::Float(64));
            let factor = b.const_float(180.0 / PI);
            let result = b.mul(atan2_val, factor);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// sqr(x: f64) -> f64  =  x * x
// ---------------------------------------------------------------------------

fn attach_body_sqr(module: &mut Module) {
    attach_runtime_body(module, "sqr", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.mul(x, x);
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// power(base: f64, exp: f64) -> f64  =  pow(base, exp)
// ---------------------------------------------------------------------------

fn attach_body_power(module: &mut Module) {
    attach_runtime_body(
        module,
        "power",
        &[Type::Float(64), Type::Float(64)],
        Type::Float(64),
        |b| {
            let base = b.param(0);
            let exp = b.param(1);
            let result = b.call_named("pow_f64", &[base, exp], Type::Float(64));
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// logn(n: f64, val: f64) -> f64  =  ln(val) / ln(n)
// ---------------------------------------------------------------------------

fn attach_body_logn(module: &mut Module) {
    attach_runtime_body(
        module,
        "logn",
        &[Type::Float(64), Type::Float(64)],
        Type::Float(64),
        |b| {
            let n = b.param(0);
            let val = b.param(1);
            let ln_val = b.call_named("ln_f64", &[val], Type::Float(64));
            let ln_n = b.call_named("ln_f64", &[n], Type::Float(64));
            let result = b.div(ln_val, ln_n);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// log2(x: f64) -> f64  =  log2(x)
// ---------------------------------------------------------------------------

fn attach_body_log2(module: &mut Module) {
    attach_runtime_body(module, "log2", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("log2_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// log10(x: f64) -> f64  =  log10(x)
// ---------------------------------------------------------------------------

fn attach_body_log10(module: &mut Module) {
    attach_runtime_body(module, "log10", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("log10_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// exp(x: f64) -> f64  =  e^x
// ---------------------------------------------------------------------------

fn attach_body_exp(module: &mut Module) {
    attach_runtime_body(module, "exp", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("exp_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// clamp(val: f64, min: f64, max: f64) -> f64  =  min_f64(max_f64(val, min), max)
// ---------------------------------------------------------------------------

fn attach_body_clamp(module: &mut Module) {
    attach_runtime_body(
        module,
        "clamp",
        &[Type::Float(64), Type::Float(64), Type::Float(64)],
        Type::Float(64),
        |b| {
            let val = b.param(0);
            let lo = b.param(1);
            let hi = b.param(2);
            let clamped_lo = b.call_named("max_f64", &[val, lo], Type::Float(64));
            let result = b.call_named("min_f64", &[clamped_lo, hi], Type::Float(64));
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// lerp(a: f64, b: f64, amt: f64) -> f64  =  a * (1 - amt) + b * amt
// ---------------------------------------------------------------------------

fn attach_body_lerp(module: &mut Module) {
    attach_runtime_body(
        module,
        "lerp",
        &[Type::Float(64), Type::Float(64), Type::Float(64)],
        Type::Float(64),
        |b| {
            let a = b.param(0);
            let bv = b.param(1);
            let amt = b.param(2);

            // a * (1 - amt) + b * amt
            let one = b.const_float(1.0);
            let one_minus_amt = b.sub(one, amt);
            let a_part = b.mul(a, one_minus_amt);
            let b_part = b.mul(bv, amt);
            let result = b.add(a_part, b_part);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// abs(x: f64) -> f64  =  abs(x)
// ---------------------------------------------------------------------------

fn attach_body_abs(module: &mut Module) {
    attach_runtime_body(module, "abs", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("abs_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// floor(x: f64) -> f64  =  floor(x)
// ---------------------------------------------------------------------------

fn attach_body_floor(module: &mut Module) {
    attach_runtime_body(module, "floor", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("floor_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// ceil(x: f64) -> f64  =  ceil(x)
// ---------------------------------------------------------------------------

fn attach_body_ceil(module: &mut Module) {
    attach_runtime_body(module, "ceil", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("ceil_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// round(x: f64) -> f64  =  round(x)
//
// GML round uses round-half-away-from-zero, same as Math.round for positive
// values and mirrored for negative.  round_f64 maps to Math.round.
// ---------------------------------------------------------------------------

fn attach_body_round(module: &mut Module) {
    attach_runtime_body(module, "round", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("round_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// sign(x: f64) -> f64  =  sign(x)  [returns -1, 0, or 1]
// ---------------------------------------------------------------------------

fn attach_body_sign(module: &mut Module) {
    attach_runtime_body(module, "sign", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("sign_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// sqrt(x: f64) -> f64  =  sqrt(x)
// ---------------------------------------------------------------------------

fn attach_body_sqrt(module: &mut Module) {
    attach_runtime_body(module, "sqrt", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("sqrt_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// arctan(x: f64) -> f64  =  atan(x)  [result in radians]
// ---------------------------------------------------------------------------

fn attach_body_arctan(module: &mut Module) {
    attach_runtime_body(module, "arctan", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("atan_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// frac(x: f64) -> f64  =  x - trunc(x)
//
// Returns the fractional part of x (the digits after the decimal point).
// For negative values, e.g. frac(-3.7) = -3.7 - (-3.0) = -0.7.
// ---------------------------------------------------------------------------

fn attach_body_frac(module: &mut Module) {
    attach_runtime_body(module, "frac", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let trunc_val = b.call_named("trunc_f64", &[x], Type::Float(64));
        let result = b.sub(x, trunc_val);
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// dot_product(x1, y1, x2, y2: f64) -> f64  =  x1*x2 + y1*y2
// ---------------------------------------------------------------------------

fn attach_body_dot_product(module: &mut Module) {
    attach_runtime_body(
        module,
        "dot_product",
        &[
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
        ],
        Type::Float(64),
        |b| {
            let x1 = b.param(0);
            let y1 = b.param(1);
            let x2 = b.param(2);
            let y2 = b.param(3);

            let x_term = b.mul(x1, x2);
            let y_term = b.mul(y1, y2);
            let result = b.add(x_term, y_term);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// dot_product_3d(x1, y1, z1, x2, y2, z2: f64) -> f64  =  x1*x2 + y1*y2 + z1*z2
// ---------------------------------------------------------------------------

fn attach_body_dot_product_3d(module: &mut Module) {
    attach_runtime_body(
        module,
        "dot_product_3d",
        &[
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
        ],
        Type::Float(64),
        |b| {
            let x1 = b.param(0);
            let y1 = b.param(1);
            let z1 = b.param(2);
            let x2 = b.param(3);
            let y2 = b.param(4);
            let z2 = b.param(5);

            let x_term = b.mul(x1, x2);
            let y_term = b.mul(y1, y2);
            let z_term = b.mul(z1, z2);
            let xy = b.add(x_term, y_term);
            let result = b.add(xy, z_term);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// color_get_red(color: f64) -> f64  =  color & 0xFF
//
// GML colors use BGR byte order; red is in the low byte.
// ---------------------------------------------------------------------------

fn attach_body_color_get_red(module: &mut Module) {
    attach_runtime_body(
        module,
        "color_get_red",
        &[Type::Float(64)],
        Type::Float(64),
        |b| {
            let color = b.param(0);
            let mask = b.const_float(255.0);
            let result = b.bit_and(color, mask);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// color_get_green(color: f64) -> f64  =  (color >> 8) & 0xFF
//
// Green occupies the middle byte.
// ---------------------------------------------------------------------------

fn attach_body_color_get_green(module: &mut Module) {
    attach_runtime_body(
        module,
        "color_get_green",
        &[Type::Float(64)],
        Type::Float(64),
        |b| {
            let color = b.param(0);
            let shift = b.const_float(8.0);
            let shifted = b.shr(color, shift);
            let mask = b.const_float(255.0);
            let result = b.bit_and(shifted, mask);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// color_get_blue(color: f64) -> f64  =  color >> 16
//
// Blue occupies the high byte; no mask needed after shifting.
// ---------------------------------------------------------------------------

fn attach_body_color_get_blue(module: &mut Module) {
    attach_runtime_body(
        module,
        "color_get_blue",
        &[Type::Float(64)],
        Type::Float(64),
        |b| {
            let color = b.param(0);
            let shift = b.const_float(16.0);
            let result = b.shr(color, shift);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// make_color_rgb(r: f64, g: f64, b: f64) -> f64  =  (b << 16) | (g << 8) | r
//
// Packs RGB components into a GML BGR color value.
// ---------------------------------------------------------------------------

fn attach_body_make_color_rgb(module: &mut Module) {
    attach_runtime_body(
        module,
        "make_color_rgb",
        &[Type::Float(64), Type::Float(64), Type::Float(64)],
        Type::Float(64),
        |b| {
            let r = b.param(0);
            let g = b.param(1);
            let bv = b.param(2);

            let shift16 = b.const_float(16.0);
            let shift8 = b.const_float(8.0);
            let b_shifted = b.shl(bv, shift16);
            let g_shifted = b.shl(g, shift8);
            let bg = b.bit_or(b_shifted, g_shifted);
            let result = b.bit_or(bg, r);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// merge_color(col1: f64, col2: f64, amount: f64) -> f64
//
// Blend two BGR colors by linearly interpolating each channel:
//   make_color_rgb(
//     round(red(col1)*(1-amount) + red(col2)*amount),
//     round(green(col1)*(1-amount) + green(col2)*amount),
//     round(blue(col1)*(1-amount) + blue(col2)*amount),
//   )
// ---------------------------------------------------------------------------

fn attach_body_merge_color(module: &mut Module) {
    attach_runtime_body(
        module,
        "merge_color",
        &[Type::Float(64), Type::Float(64), Type::Float(64)],
        Type::Float(64),
        |b| {
            let col1 = b.param(0);
            let col2 = b.param(1);
            let amt = b.param(2);

            let one = b.const_float(1.0);
            let one_minus_amt = b.sub(one, amt);

            // Red channel
            let r1 = b.call_named("color_get_red", &[col1], Type::Float(64));
            let r2 = b.call_named("color_get_red", &[col2], Type::Float(64));
            let r1_part = b.mul(r1, one_minus_amt);
            let r2_part = b.mul(r2, amt);
            let r_blend = b.add(r1_part, r2_part);
            let r_out = b.call_named("round_f64", &[r_blend], Type::Float(64));

            // Green channel
            let g1 = b.call_named("color_get_green", &[col1], Type::Float(64));
            let g2 = b.call_named("color_get_green", &[col2], Type::Float(64));
            let g1_part = b.mul(g1, one_minus_amt);
            let g2_part = b.mul(g2, amt);
            let g_blend = b.add(g1_part, g2_part);
            let g_out = b.call_named("round_f64", &[g_blend], Type::Float(64));

            // Blue channel
            let b1 = b.call_named("color_get_blue", &[col1], Type::Float(64));
            let b2 = b.call_named("color_get_blue", &[col2], Type::Float(64));
            let b1_part = b.mul(b1, one_minus_amt);
            let b2_part = b.mul(b2, amt);
            let b_blend = b.add(b1_part, b2_part);
            let bv_out = b.call_named("round_f64", &[b_blend], Type::Float(64));

            let result = b.call_named("make_color_rgb", &[r_out, g_out, bv_out], Type::Float(64));
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// color_get_value(color: f64) -> f64
//
// Returns the HSV "value" (brightness) of a GML BGR color in the range 0–255:
//   r = (color & 0xFF) / 255
//   g = ((color >> 8) & 0xFF) / 255
//   b = (color >> 16) / 255
//   round(max(r, g, b) * 255)
// ---------------------------------------------------------------------------

fn attach_body_color_get_value(module: &mut Module) {
    attach_runtime_body(
        module,
        "color_get_value",
        &[Type::Float(64)],
        Type::Float(64),
        |b| {
            let color = b.param(0);

            let c255 = b.const_float(255.0);

            let r_raw = b.call_named("color_get_red", &[color], Type::Float(64));
            let g_raw = b.call_named("color_get_green", &[color], Type::Float(64));
            let bv_raw = b.call_named("color_get_blue", &[color], Type::Float(64));
            let r = b.div(r_raw, c255);
            let g = b.div(g_raw, c255);
            let bv = b.div(bv_raw, c255);

            let max_rg = b.call_named("max_f64", &[r, g], Type::Float(64));
            let max_rgb = b.call_named("max_f64", &[max_rg, bv], Type::Float(64));

            let scaled = b.mul(max_rgb, c255);
            let result = b.call_named("round_f64", &[scaled], Type::Float(64));
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// color_get_saturation(color: f64) -> f64
//
// Returns the HSV saturation in 0–255:
//   r = (color & 0xFF) / 255
//   g = ((color >> 8) & 0xFF) / 255
//   b = (color >> 16) / 255
//   max = max(r, g, b);  min = min(r, g, b)
//   if max <= 0 { return 0 }   [max >= 0 always, so this equals max === 0]
//   round(((max - min) / max) * 255)
// ---------------------------------------------------------------------------

fn attach_body_color_get_saturation(module: &mut Module) {
    attach_runtime_body(
        module,
        "color_get_saturation",
        &[Type::Float(64)],
        Type::Float(64),
        |b| {
            let color = b.param(0);

            let c255 = b.const_float(255.0);
            let zero = b.const_float(0.0);

            let r_raw = b.call_named("color_get_red", &[color], Type::Float(64));
            let g_raw = b.call_named("color_get_green", &[color], Type::Float(64));
            let bv_raw = b.call_named("color_get_blue", &[color], Type::Float(64));
            let r = b.div(r_raw, c255);
            let g = b.div(g_raw, c255);
            let bv = b.div(bv_raw, c255);

            let max_rg = b.call_named("max_f64", &[r, g], Type::Float(64));
            let max_rgb = b.call_named("max_f64", &[max_rg, bv], Type::Float(64));
            let min_rg = b.call_named("min_f64", &[r, g], Type::Float(64));
            let min_rgb = b.call_named("min_f64", &[min_rg, bv], Type::Float(64));

            // if max <= 0 { return 0 }  (max >= 0 always, so this equals max === 0)
            let max_le_zero = b.cmp(CmpKind::Le, max_rgb, zero);
            let ret_zero_block = b.create_block();
            let cont_block = b.create_block();
            b.br_if(max_le_zero, ret_zero_block, &[], cont_block, &[]);

            b.switch_to_block(ret_zero_block);
            b.ret(Some(zero));

            b.switch_to_block(cont_block);
            let d = b.sub(max_rgb, min_rgb);
            let sat = b.div(d, max_rgb);
            let scaled = b.mul(sat, c255);
            let result = b.call_named("round_f64", &[scaled], Type::Float(64));
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// color_get_hue(color: f64) -> f64
//
// Returns the HSV hue in 0–255 (GML maps 0–360° to 0–255).
//
//   r = (color & 0xFF) / 255
//   g = ((color >> 8) & 0xFF) / 255
//   b = (color >> 16) / 255
//   max = max(r, g, b);  min = min(r, g, b);  d = max - min
//   if d <= 0 { return 0 }
//   // 3-way select (avoid CmpKind::Eq on floats derived from bitwise ops):
//   if r >= g && r >= b: h = ((g - b) / d) % 6
//   else if g >= b:      h = (b - r) / d + 2
//   else:                h = (r - g) / d + 4
//   h = round(h * 255 / 6)
//   if h < 0 { h += 255 }
//   return h
// ---------------------------------------------------------------------------

fn attach_body_color_get_hue(module: &mut Module) {
    attach_runtime_body(
        module,
        "color_get_hue",
        &[Type::Float(64)],
        Type::Float(64),
        |b| {
            let color = b.param(0);

            let c255 = b.const_float(255.0);
            let zero = b.const_float(0.0);

            // Extract r, g, b as fractions in [0, 1].
            let r_raw = b.call_named("color_get_red", &[color], Type::Float(64));
            let g_raw = b.call_named("color_get_green", &[color], Type::Float(64));
            let bv_raw = b.call_named("color_get_blue", &[color], Type::Float(64));
            let r = b.div(r_raw, c255);
            let g = b.div(g_raw, c255);
            let bv = b.div(bv_raw, c255);

            let max_rg = b.call_named("max_f64", &[r, g], Type::Float(64));
            let max_rgb = b.call_named("max_f64", &[max_rg, bv], Type::Float(64));
            let min_rg = b.call_named("min_f64", &[r, g], Type::Float(64));
            let min_rgb = b.call_named("min_f64", &[min_rg, bv], Type::Float(64));
            let d = b.sub(max_rgb, min_rgb);

            // if d <= 0 { return 0 }  (d >= 0 always, so this equals d === 0)
            let d_le_zero = b.cmp(CmpKind::Le, d, zero);
            let ret_zero_block = b.create_block();
            let branch_r_check = b.create_block();
            b.br_if(d_le_zero, ret_zero_block, &[], branch_r_check, &[]);

            b.switch_to_block(ret_zero_block);
            b.ret(Some(zero));

            // 3-way branch: is r the maximum channel?
            b.switch_to_block(branch_r_check);
            let r_ge_g = b.cmp(CmpKind::Ge, r, g);
            let r_ge_b = b.cmp(CmpKind::Ge, r, bv);
            let r_is_max = b.call_named("and_bool", &[r_ge_g, r_ge_b], Type::Bool);

            let (merge_block, h_params) = b.create_block_with_params(&[Type::Float(64)]);
            let block_r = b.create_block();
            let block_not_r = b.create_block();
            b.br_if(r_is_max, block_r, &[], block_not_r, &[]);

            // Branch: r is max → h = ((g - b) / d) % 6
            b.switch_to_block(block_r);
            let c6 = b.const_float(6.0);
            let g_minus_b = b.sub(g, bv);
            let h_r_raw = b.div(g_minus_b, d);
            let h_r = b.rem(h_r_raw, c6);
            b.br(merge_block, &[h_r]);

            // Branch: r is not max — is g the max? (check g >= b)
            b.switch_to_block(block_not_r);
            let g_ge_b = b.cmp(CmpKind::Ge, g, bv);
            let block_g = b.create_block();
            let block_bv = b.create_block();
            b.br_if(g_ge_b, block_g, &[], block_bv, &[]);

            // Branch: g is max → h = (b - r) / d + 2
            b.switch_to_block(block_g);
            let two = b.const_float(2.0);
            let bv_minus_r = b.sub(bv, r);
            let h_g_div = b.div(bv_minus_r, d);
            let h_g = b.add(h_g_div, two);
            b.br(merge_block, &[h_g]);

            // Branch: b is max → h = (r - g) / d + 4
            b.switch_to_block(block_bv);
            let four = b.const_float(4.0);
            let r_minus_g = b.sub(r, g);
            let h_bv_div = b.div(r_minus_g, d);
            let h_bv = b.add(h_bv_div, four);
            b.br(merge_block, &[h_bv]);

            // Merge: h_raw is the block param from whichever branch.
            b.switch_to_block(merge_block);
            let h_raw = h_params[0];

            // h = round(h_raw * 255 / 6)
            let c255_over_6 = b.const_float(255.0 / 6.0);
            let h_scaled = b.mul(h_raw, c255_over_6);
            let h_rounded = b.call_named("round_f64", &[h_scaled], Type::Float(64));

            // if h < 0 { h += 255 }
            let h_lt_zero = b.cmp(CmpKind::Lt, h_rounded, zero);
            let (final_block, final_params) = b.create_block_with_params(&[Type::Float(64)]);
            let add_255_block = b.create_block();
            b.br_if(h_lt_zero, add_255_block, &[], final_block, &[h_rounded]);

            b.switch_to_block(add_255_block);
            let h_plus_255 = b.add(h_rounded, c255);
            b.br(final_block, &[h_plus_255]);

            b.switch_to_block(final_block);
            let result = final_params[0];
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// make_color_hsv(h: f64, s: f64, v: f64) -> f64
//
// Converts HSV (0–255 each) to a GML BGR color.
//   hf = (h / 255) * 6    — hue in [0, 6)
//   sf = s / 255
//   vf = v / 255
//   c  = vf * sf           — chroma
//   x  = c * (1 - |hf % 2 - 1|)
//   m  = vf - c
//   6-way branch on hf:
//     hf < 1 → (r=c, g=x, b=0)
//     hf < 2 → (r=x, g=c, b=0)
//     hf < 3 → (r=0, g=c, b=x)
//     hf < 4 → (r=0, g=x, b=c)
//     hf < 5 → (r=x, g=0, b=c)
//     else   → (r=c, g=0, b=x)
//   return make_color_rgb(round((r+m)*255), round((g+m)*255), round((b+m)*255))
// ---------------------------------------------------------------------------

fn attach_body_make_color_hsv(module: &mut Module) {
    attach_runtime_body(
        module,
        "make_color_hsv",
        &[Type::Float(64), Type::Float(64), Type::Float(64)],
        Type::Float(64),
        |b| {
            let h = b.param(0);
            let s = b.param(1);
            let v = b.param(2);

            let c255 = b.const_float(255.0);
            let c6 = b.const_float(6.0);

            // Normalize inputs.
            let h_div = b.div(h, c255);
            let hf = b.mul(h_div, c6);
            let sf = b.div(s, c255);
            let vf = b.div(v, c255);

            // c = vf * sf;  x = c * (1 - |hf % 2 - 1|);  m = vf - c
            let cv = b.mul(vf, sf);
            let c2_hsv = b.const_float(2.0);
            let hf_mod2 = b.rem(hf, c2_hsv);
            let c1_hsv = b.const_float(1.0);
            let hf_mod2_m1 = b.sub(hf_mod2, c1_hsv);
            let abs_val = b.call_named("abs_f64", &[hf_mod2_m1], Type::Float(64));
            let c1_hsv2 = b.const_float(1.0);
            let one_minus_abs = b.sub(c1_hsv2, abs_val);
            let x = b.mul(cv, one_minus_abs);
            let m = b.sub(vf, cv);

            // Merge block collects (r, g, b) from the 6 branches.
            let (merge_block, rgb_params) =
                b.create_block_with_params(&[Type::Float(64), Type::Float(64), Type::Float(64)]);

            let c1 = b.const_float(1.0);
            let c2 = b.const_float(2.0);
            let c3 = b.const_float(3.0);
            let c4 = b.const_float(4.0);
            let c5 = b.const_float(5.0);
            let zero = b.const_float(0.0);

            // hf < 1?
            let hf_lt1 = b.cmp(CmpKind::Lt, hf, c1);
            let blk0 = b.create_block();
            let blk_ge1 = b.create_block();
            b.br_if(hf_lt1, blk0, &[], blk_ge1, &[]);

            // Branch 0: hf < 1 → r=c, g=x, b=0
            b.switch_to_block(blk0);
            b.br(merge_block, &[cv, x, zero]);

            // hf >= 1: hf < 2?
            b.switch_to_block(blk_ge1);
            let hf_lt2 = b.cmp(CmpKind::Lt, hf, c2);
            let blk1 = b.create_block();
            let blk_ge2 = b.create_block();
            b.br_if(hf_lt2, blk1, &[], blk_ge2, &[]);

            // Branch 1: 1 <= hf < 2 → r=x, g=c, b=0
            b.switch_to_block(blk1);
            b.br(merge_block, &[x, cv, zero]);

            // hf >= 2: hf < 3?
            b.switch_to_block(blk_ge2);
            let hf_lt3 = b.cmp(CmpKind::Lt, hf, c3);
            let blk2 = b.create_block();
            let blk_ge3 = b.create_block();
            b.br_if(hf_lt3, blk2, &[], blk_ge3, &[]);

            // Branch 2: 2 <= hf < 3 → r=0, g=c, b=x
            b.switch_to_block(blk2);
            b.br(merge_block, &[zero, cv, x]);

            // hf >= 3: hf < 4?
            b.switch_to_block(blk_ge3);
            let hf_lt4 = b.cmp(CmpKind::Lt, hf, c4);
            let blk3 = b.create_block();
            let blk_ge4 = b.create_block();
            b.br_if(hf_lt4, blk3, &[], blk_ge4, &[]);

            // Branch 3: 3 <= hf < 4 → r=0, g=x, b=c
            b.switch_to_block(blk3);
            b.br(merge_block, &[zero, x, cv]);

            // hf >= 4: hf < 5?
            b.switch_to_block(blk_ge4);
            let hf_lt5 = b.cmp(CmpKind::Lt, hf, c5);
            let blk4 = b.create_block();
            let blk5 = b.create_block();
            b.br_if(hf_lt5, blk4, &[], blk5, &[]);

            // Branch 4: 4 <= hf < 5 → r=x, g=0, b=c
            b.switch_to_block(blk4);
            b.br(merge_block, &[x, zero, cv]);

            // Branch 5: hf >= 5 → r=c, g=0, b=x
            b.switch_to_block(blk5);
            b.br(merge_block, &[cv, zero, x]);

            // Merge: apply m offset and pack into BGR.
            b.switch_to_block(merge_block);
            let r_out = rgb_params[0];
            let g_out = rgb_params[1];
            let bv_out = rgb_params[2];

            let r_plus_m = b.add(r_out, m);
            let r_scaled = b.mul(r_plus_m, c255);
            let r_final = b.call_named("round_f64", &[r_scaled], Type::Float(64));
            let g_plus_m = b.add(g_out, m);
            let g_scaled = b.mul(g_plus_m, c255);
            let g_final = b.call_named("round_f64", &[g_scaled], Type::Float(64));
            let bv_plus_m = b.add(bv_out, m);
            let bv_scaled = b.mul(bv_plus_m, c255);
            let b_final = b.call_named("round_f64", &[bv_scaled], Type::Float(64));

            let result = b.call_named(
                "make_color_rgb",
                &[r_final, g_final, b_final],
                Type::Float(64),
            );
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_length(s: String) -> Float(64)  =  s.length
// ---------------------------------------------------------------------------

fn attach_body_string_length(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_length",
        &[Type::String],
        Type::Float(64),
        |b| {
            let s = b.param(0);
            let result = b.call_named("string_length_str", &[s], Type::Float(64));
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_upper(s: String) -> String  =  s.toUpperCase()
// ---------------------------------------------------------------------------

fn attach_body_string_upper(module: &mut Module) {
    attach_runtime_body(module, "string_upper", &[Type::String], Type::String, |b| {
        let s = b.param(0);
        let result = b.call_named("string_upper_str", &[s], Type::String);
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// string_lower(s: String) -> String  =  s.toLowerCase()
// ---------------------------------------------------------------------------

fn attach_body_string_lower(module: &mut Module) {
    attach_runtime_body(module, "string_lower", &[Type::String], Type::String, |b| {
        let s = b.param(0);
        let result = b.call_named("string_lower_str", &[s], Type::String);
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// string_char_at(s: String, index: Float(64)) -> String
//   GML is 1-based: s.charAt(index - 1)
// ---------------------------------------------------------------------------

fn attach_body_string_char_at(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_char_at",
        &[Type::String, Type::Float(64)],
        Type::String,
        |b| {
            let s = b.param(0);
            let index = b.param(1);

            let one = b.const_float(1.0);
            let idx_zero = b.sub(index, one);
            let result = b.call_named("string_char_at_str", &[s, idx_zero], Type::String);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_copy(s: String, index: Float(64), count: Float(64)) -> String
//   GML is 1-based: s.slice(index-1, index-1+count)
// ---------------------------------------------------------------------------

fn attach_body_string_copy(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_copy",
        &[Type::String, Type::Float(64), Type::Float(64)],
        Type::String,
        |b| {
            let s = b.param(0);
            let index = b.param(1);
            let count = b.param(2);

            let one = b.const_float(1.0);
            let start = b.sub(index, one); // index - 1  (0-based start)
            let end = b.add(start, count); // start + count  (0-based exclusive end)
            let result = b.call_named("string_slice_str", &[s, start, end], Type::String);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_pos(substr: String, s: String) -> Float(64)
//   Returns 1-based position, 0 if not found.
//   JS indexOf returns 0-based (-1 if not found); adding 1 gives GML semantics:
//   -1 + 1 = 0 (not found), n + 1 = 1-based position.
// ---------------------------------------------------------------------------

fn attach_body_string_pos(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_pos",
        &[Type::String, Type::String],
        Type::Float(64),
        |b| {
            let substr = b.param(0);
            let s = b.param(1);

            // string_index_of_str(needle, haystack) -> 0-based index or -1
            let idx = b.call_named("string_index_of_str", &[substr, s], Type::Float(64));
            let one = b.const_float(1.0);
            let result = b.add(idx, one);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_delete(s: String, index: Float(64), count: Float(64)) -> String
//   s.slice(0, index-1) + s.slice(index-1+count, length)
// ---------------------------------------------------------------------------

fn attach_body_string_delete(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_delete",
        &[Type::String, Type::Float(64), Type::Float(64)],
        Type::String,
        |b| {
            let s = b.param(0);
            let index = b.param(1);
            let count = b.param(2);

            let zero = b.const_float(0.0);
            let one = b.const_float(1.0);
            let idx_minus1 = b.sub(index, one); // index - 1  (0-based)
            let tail_start = b.add(idx_minus1, count); // index - 1 + count
            let len = b.call_named("string_length_str", &[s], Type::Float(64));

            let head = b.call_named("string_slice_str", &[s, zero, idx_minus1], Type::String);
            let tail = b.call_named("string_slice_str", &[s, tail_start, len], Type::String);
            let result = b.call_named("concat_str", &[head, tail], Type::String);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_insert(substr: String, s: String, index: Float(64)) -> String
//   s.slice(0, index-1) + substr + s.slice(index-1)
// ---------------------------------------------------------------------------

fn attach_body_string_insert(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_insert",
        &[Type::String, Type::String, Type::Float(64)],
        Type::String,
        |b| {
            let substr = b.param(0);
            let s = b.param(1);
            let index = b.param(2);

            let zero = b.const_float(0.0);
            let one = b.const_float(1.0);
            let idx_minus1 = b.sub(index, one); // index - 1  (0-based)
            let len = b.call_named("string_length_str", &[s], Type::Float(64));

            let head = b.call_named("string_slice_str", &[s, zero, idx_minus1], Type::String);
            let tail = b.call_named("string_slice_str", &[s, idx_minus1, len], Type::String);
            // head + substr + tail  (two string concatenations)
            let head_sub = b.call_named("concat_str", &[head, substr], Type::String);
            let result = b.call_named("concat_str", &[head_sub, tail], Type::String);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_replace_all(str: String, substr: String, newstr: String) -> String
//   str.split(substr).join(newstr)
// ---------------------------------------------------------------------------

fn attach_body_string_replace_all(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_replace_all",
        &[Type::String, Type::String, Type::String],
        Type::String,
        |b| {
            let content = b.param(0);
            let find = b.param(1);
            let replace = b.param(2);

            let arr_ty = Type::Array(Box::new(Type::String));
            let parts = b.call_named("string_split_str", &[content, find], arr_ty);
            let result = b.call_method(parts, "join", &[replace], Type::String);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_count(substr: String, s: String) -> Float(64)
//   s.split(substr).length - 1
// ---------------------------------------------------------------------------

fn attach_body_string_count(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_count",
        &[Type::String, Type::String],
        Type::Float(64),
        |b| {
            let substr = b.param(0);
            let s = b.param(1);

            let arr_ty = Type::Array(Box::new(Type::String));
            let parts = b.call_named("string_split_str", &[s, substr], arr_ty);
            let len = b.get_field(parts, "length", Type::Float(64));
            let one = b.const_float(1.0);
            let result = b.sub(len, one);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_ord_at(str: String, index: Float(64)) -> Float(64)
//   GML: char code at 1-based index
//   JS equivalent: str.charCodeAt(index - 1)
// ---------------------------------------------------------------------------

fn attach_body_string_ord_at(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_ord_at",
        &[Type::String, Type::Float(64)],
        Type::Float(64),
        |b| {
            let s = b.param(0);
            let idx = b.param(1);

            let one = b.const_float(1.0);
            let idx0 = b.sub(idx, one); // convert 1-based GML index to 0-based JS
            let result = b.call_named("string_char_code_at_str", &[s, idx0], Type::Float(64));
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_repeat(str: String, count: Float(64)) -> String
//   JS: str.repeat(count)
// ---------------------------------------------------------------------------

fn attach_body_string_repeat(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_repeat",
        &[Type::String, Type::Float(64)],
        Type::String,
        |b| {
            let s = b.param(0);
            let n = b.param(1);
            let result = b.call_named("string_repeat_str", &[s, n], Type::String);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_replace(str: String, substr: String, newstr: String) -> String
//   GML: replaces FIRST occurrence only
//   JS: str.replace(substr, newstr)
// ---------------------------------------------------------------------------

fn attach_body_string_replace(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_replace",
        &[Type::String, Type::String, Type::String],
        Type::String,
        |b| {
            let s = b.param(0);
            let sub = b.param(1);
            let new = b.param(2);
            let result = b.call_named("string_replace_first_str", &[s, sub, new], Type::String);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_hash_to_newline(string: String) -> String
//   GML: replaces '#' with newline
//   JS: str.split('#').join('\n')
// ---------------------------------------------------------------------------

fn attach_body_string_hash_to_newline(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_hash_to_newline",
        &[Type::String],
        Type::String,
        |b| {
            let s = b.param(0);
            let hash = b.const_string("#");
            let newline = b.const_string("\n");
            let arr_ty = Type::Array(Box::new(Type::String));
            let parts = b.call_named("string_split_str", &[s, hash], arr_ty);
            let result = b.call_method(parts, "join", &[newline], Type::String);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// string_trim(str: String, substr: Unknown) -> String
//   GML: trims whitespace (optional 2nd param ignored for whitespace-trim form)
//   JS: str.trim()
// ---------------------------------------------------------------------------

fn attach_body_string_trim(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_trim",
        &[Type::String, Type::Unknown],
        Type::String,
        |b| {
            let s = b.param(0);
            b.param(1); // substr — unused in whitespace-trim form
            let result = b.call_named("string_trim_str", &[s], Type::String);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// array_length(array: Array(Unknown)) -> Float(64)
//   JS: arr.length
// ---------------------------------------------------------------------------

fn attach_body_array_length(module: &mut Module) {
    attach_runtime_body(
        module,
        "array_length",
        &[Type::Array(Box::new(Type::Unknown))],
        Type::Float(64),
        |b| {
            let arr = b.param(0);
            let result = b.call_named("array_length_arr", &[arr], Type::Float(64));
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// array_length_1d(array: Array(Unknown)) -> Float(64)
//   Alias for array_length; identical behaviour.
//   JS: arr.length
// ---------------------------------------------------------------------------

fn attach_body_array_length_1d(module: &mut Module) {
    attach_runtime_body(
        module,
        "array_length_1d",
        &[Type::Array(Box::new(Type::Unknown))],
        Type::Float(64),
        |b| {
            let arr = b.param(0);
            let result = b.call_named("array_length_arr", &[arr], Type::Float(64));
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// array_contains(array: Array(Unknown), value: Unknown) -> Bool
//   GML: checks if value is in array
//   JS: arr.includes(value)
// ---------------------------------------------------------------------------

fn attach_body_array_contains(module: &mut Module) {
    attach_runtime_body(
        module,
        "array_contains",
        &[Type::Array(Box::new(Type::Unknown)), Type::Unknown],
        Type::Bool,
        |b| {
            let arr = b.param(0);
            let val = b.param(1);
            let result = b.call_named("array_contains_arr", &[arr, val], Type::Bool);
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// sin(x: f64) -> f64  — radian version
//   JS: Math.sin(x)
// ---------------------------------------------------------------------------

fn attach_body_sin(module: &mut Module) {
    attach_runtime_body(module, "sin", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("sin_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// cos(x: f64) -> f64  — radian version
//   JS: Math.cos(x)
// ---------------------------------------------------------------------------

fn attach_body_cos(module: &mut Module) {
    attach_runtime_body(module, "cos", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("cos_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// tan(x: f64) -> f64  — radian version
//   JS: Math.tan(x)
// ---------------------------------------------------------------------------

fn attach_body_tan(module: &mut Module) {
    attach_runtime_body(module, "tan", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("tan_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// arcsin(x: f64) -> f64  — radian version
//   JS: Math.asin(x)
// ---------------------------------------------------------------------------

fn attach_body_arcsin(module: &mut Module) {
    attach_runtime_body(module, "arcsin", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("asin_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// arccos(x: f64) -> f64  — radian version
//   JS: Math.acos(x)
// ---------------------------------------------------------------------------

fn attach_body_arccos(module: &mut Module) {
    attach_runtime_body(module, "arccos", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("acos_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// ord(s: String) -> Float(64)
//   GML: returns the Unicode code point of the first character of s.
//   JS: s.charCodeAt(0)
// ---------------------------------------------------------------------------

fn attach_body_ord(module: &mut Module) {
    attach_runtime_body(module, "ord", &[Type::String], Type::Float(64), |b| {
        let s = b.param(0);
        let zero = b.const_float(0.0);
        let result = b.call_named("string_char_code_at_str", &[s, zero], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// string_byte_at(s: String, pos: Float(64)) -> Float(64)
//   GML: returns the byte value of the character at 1-based position pos.
//   JS: s.charCodeAt(pos - 1)
// ---------------------------------------------------------------------------

fn attach_body_string_byte_at(module: &mut Module) {
    attach_runtime_body(
        module,
        "string_byte_at",
        &[Type::String, Type::Float(64)],
        Type::Float(64),
        |b| {
            let s = b.param(0);
            let pos = b.param(1);

            let one = b.const_float(1.0);
            let pos_minus_1 = b.sub(pos, one); // convert 1-based GML index to 0-based JS
            let result = b.call_named(
                "string_char_code_at_str",
                &[s, pos_minus_1],
                Type::Float(64),
            );
            b.ret(Some(result));
        },
    );
}

// ---------------------------------------------------------------------------
// chr(n: Float(64)) -> String
//   GML: returns the character corresponding to Unicode code point n.
//   JS: String.fromCharCode(n)
// ---------------------------------------------------------------------------

fn attach_body_chr(module: &mut Module) {
    attach_runtime_body(module, "chr", &[Type::Float(64)], Type::String, |b| {
        let n = b.param(0);
        let result = b.call_named("chr_f64", &[n], Type::String);
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// ln(x: f64) -> f64  =  ln_f64(x)
// ---------------------------------------------------------------------------

fn attach_body_ln(module: &mut Module) {
    attach_runtime_body(module, "ln", &[Type::Float(64)], Type::Float(64), |b| {
        let x = b.param(0);
        let result = b.call_named("ln_f64", &[x], Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// math_get_epsilon() -> f64  =  0.00001
// ---------------------------------------------------------------------------

fn attach_body_math_get_epsilon(module: &mut Module) {
    attach_runtime_body(module, "math_get_epsilon", &[], Type::Float(64), |b| {
        let result = b.const_float(0.00001);
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// is_bool(x: Unknown) -> Bool  =  type_check(x, Bool)
// ---------------------------------------------------------------------------

fn attach_body_is_bool(module: &mut Module) {
    attach_runtime_body(module, "is_bool", &[Type::Unknown], Type::Bool, |b| {
        let x = b.param(0);
        let result = b.type_check(x, Type::Bool);
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// is_real(x: Unknown) -> Bool  =  type_check(x, Float(64))
// JS semantics: typeof val === "number"
// ---------------------------------------------------------------------------

fn attach_body_is_real(module: &mut Module) {
    attach_runtime_body(module, "is_real", &[Type::Unknown], Type::Bool, |b| {
        let x = b.param(0);
        let result = b.type_check(x, Type::Float(64));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// is_string(x: Unknown) -> Bool  =  type_check(x, String)
// JS semantics: typeof val === "string"
// ---------------------------------------------------------------------------

fn attach_body_is_string(module: &mut Module) {
    attach_runtime_body(module, "is_string", &[Type::Unknown], Type::Bool, |b| {
        let x = b.param(0);
        let result = b.type_check(x, Type::String);
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// is_undefined(x: Unknown) -> Bool  =  type_check(x, Void)
// JS semantics: typeof val === "undefined"
// ---------------------------------------------------------------------------

fn attach_body_is_undefined(module: &mut Module) {
    attach_runtime_body(module, "is_undefined", &[Type::Unknown], Type::Bool, |b| {
        let x = b.param(0);
        let result = b.type_check(x, Type::Void);
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// is_array(x: Unknown) -> Bool  =  type_check(x, Array(Unknown))
// JS semantics: Array.isArray(val)
// ---------------------------------------------------------------------------

fn attach_body_is_array(module: &mut Module) {
    attach_runtime_body(module, "is_array", &[Type::Unknown], Type::Bool, |b| {
        let x = b.param(0);
        let result = b.type_check(x, Type::Array(Box::new(Type::Unknown)));
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// is_method(x: Unknown) -> Bool  =  type_check(x, Function(...))
// JS semantics: typeof val === "function"
// ---------------------------------------------------------------------------

fn attach_body_is_method(module: &mut Module) {
    attach_runtime_body(module, "is_method", &[Type::Unknown], Type::Bool, |b| {
        let x = b.param(0);
        // Use a zero-param function type as the representative function type.
        // The backend dispatches on `Type::Function(_)` and emits `typeof x === "function"`.
        let fn_ty = Type::Function(Box::new(FunctionSig {
            params: vec![],
            return_ty: Type::Unknown,
            defaults: vec![],
            has_rest_param: false,
            param_lower_bounds: vec![],
        }));
        let result = b.type_check(x, fn_ty);
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// is_struct(x: Unknown) -> Bool  =  is_struct_unknown(x)
// JS semantics: typeof val === "object" && val != null && !Array.isArray(val)
// ---------------------------------------------------------------------------

fn attach_body_is_struct(module: &mut Module) {
    attach_runtime_body(module, "is_struct", &[Type::Unknown], Type::Bool, |b| {
        let x = b.param(0);
        let result = b.call_named("is_struct_unknown", &[x], Type::Bool);
        b.ret(Some(result));
    });
}

// ---------------------------------------------------------------------------
// real(s: String) -> Float(64)  =  to_number_str(s)
// ---------------------------------------------------------------------------

fn attach_body_real(module: &mut Module) {
    attach_runtime_body(module, "real", &[Type::String], Type::Float(64), |b| {
        let s = b.param(0);
        let result = b.call_named("to_number_str", &[s], Type::Float(64));
        b.ret(Some(result));
    });
}
