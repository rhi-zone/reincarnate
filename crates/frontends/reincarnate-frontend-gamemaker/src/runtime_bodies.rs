use std::f64::consts::PI;

use reincarnate_core::ir::builder::FunctionBuilder;
use reincarnate_core::ir::func::Visibility;
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
/// The bodies use only `builtin.*_f64` calls and `Const(Float)` values, so they
/// are legal IR for any pipeline stage that runs after registration.
pub fn register_runtime_bodies(module: &mut Module) {
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
    attach_body_color_get_green(module);
    attach_body_color_get_blue(module);
    attach_body_make_color_rgb(module);
    attach_body_colour_get_red(module);
    attach_body_colour_get_green(module);
    attach_body_colour_get_blue(module);
    attach_body_make_colour_rgb(module);
    attach_body_merge_color(module);
    attach_body_merge_colour(module);
    attach_body_color_get_value(module);
    attach_body_colour_get_value(module);
    attach_body_color_get_saturation(module);
    attach_body_colour_get_saturation(module);
    attach_body_color_get_hue(module);
    attach_body_colour_get_hue(module);
    attach_body_make_color_hsv(module);
    attach_body_make_colour_hsv(module);
}

// ---------------------------------------------------------------------------
// lengthdir_x(len: f64, dir: f64) -> f64  =  len * cos(dir * π/180)
// ---------------------------------------------------------------------------

fn attach_body_lengthdir_x(module: &mut Module) {
    let fid = match module.lookup_runtime("lengthdir_x") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("lengthdir_x", sig, Visibility::Public);
    let len = b.param(0);
    let dir = b.param(1);

    let pi_over_180 = b.const_float(PI / 180.0);
    let dir_rad = b.mul(dir, pi_over_180);
    let cos_val = b.call("builtin.cos_f64", &[dir_rad], Type::Float(64));
    let result = b.mul(len, cos_val);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// lengthdir_y(len: f64, dir: f64) -> f64  =  len * sin(dir * π/180)
// ---------------------------------------------------------------------------

fn attach_body_lengthdir_y(module: &mut Module) {
    let fid = match module.lookup_runtime("lengthdir_y") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    // GML uses a y-down coordinate system where angle 0 points right and
    // increases counter-clockwise.  The GameMaker manual defines
    // `lengthdir_y` as `len * -sin(dir * π/180)` because increasing y goes
    // down, which flips the vertical component.
    let mut b = FunctionBuilder::new("lengthdir_y", sig, Visibility::Public);
    let len = b.param(0);
    let dir = b.param(1);

    let pi_over_180 = b.const_float(PI / 180.0);
    let dir_rad = b.mul(dir, pi_over_180);
    let sin_val = b.call("builtin.sin_f64", &[dir_rad], Type::Float(64));
    let neg_sin = b.neg(sin_val);
    let result = b.mul(len, neg_sin);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// point_distance(x1, y1, x2, y2: f64) -> f64  =  hypot(x2-x1, y2-y1)
// ---------------------------------------------------------------------------

fn attach_body_point_distance(module: &mut Module) {
    let fid = match module.lookup_runtime("point_distance") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
        ],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("point_distance", sig, Visibility::Public);
    let x1 = b.param(0);
    let y1 = b.param(1);
    let x2 = b.param(2);
    let y2 = b.param(3);

    let dx = b.sub(x2, x1);
    let dy = b.sub(y2, y1);
    let result = b.call("builtin.hypot_f64", &[dx, dy], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// degtorad(x: f64) -> f64  =  x * π/180
// ---------------------------------------------------------------------------

fn attach_body_degtorad(module: &mut Module) {
    let fid = match module.lookup_runtime("degtorad") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("degtorad", sig, Visibility::Public);
    let x = b.param(0);

    let factor = b.const_float(PI / 180.0);
    let result = b.mul(x, factor);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// radtodeg(x: f64) -> f64  =  x * 180/π
// ---------------------------------------------------------------------------

fn attach_body_radtodeg(module: &mut Module) {
    let fid = match module.lookup_runtime("radtodeg") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("radtodeg", sig, Visibility::Public);
    let x = b.param(0);

    let factor = b.const_float(180.0 / PI);
    let result = b.mul(x, factor);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// dsin(x: f64) -> f64  =  sin(x * π/180)
// ---------------------------------------------------------------------------

fn attach_body_dsin(module: &mut Module) {
    let fid = match module.lookup_runtime("dsin") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("dsin", sig, Visibility::Public);
    let x = b.param(0);

    let factor = b.const_float(PI / 180.0);
    let rad = b.mul(x, factor);
    let result = b.call("builtin.sin_f64", &[rad], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// dcos(x: f64) -> f64  =  cos(x * π/180)
// ---------------------------------------------------------------------------

fn attach_body_dcos(module: &mut Module) {
    let fid = match module.lookup_runtime("dcos") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("dcos", sig, Visibility::Public);
    let x = b.param(0);

    let factor = b.const_float(PI / 180.0);
    let rad = b.mul(x, factor);
    let result = b.call("builtin.cos_f64", &[rad], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// dtan(x: f64) -> f64  =  tan(x * π/180)
// ---------------------------------------------------------------------------

fn attach_body_dtan(module: &mut Module) {
    let fid = match module.lookup_runtime("dtan") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("dtan", sig, Visibility::Public);
    let x = b.param(0);

    let factor = b.const_float(PI / 180.0);
    let rad = b.mul(x, factor);
    let result = b.call("builtin.tan_f64", &[rad], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// darcsin(x: f64) -> f64  =  asin(x) * 180/π
// ---------------------------------------------------------------------------

fn attach_body_darcsin(module: &mut Module) {
    let fid = match module.lookup_runtime("darcsin") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("darcsin", sig, Visibility::Public);
    let x = b.param(0);

    let asin_val = b.call("builtin.asin_f64", &[x], Type::Float(64));
    let factor = b.const_float(180.0 / PI);
    let result = b.mul(asin_val, factor);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// darccos(x: f64) -> f64  =  acos(x) * 180/π
// ---------------------------------------------------------------------------

fn attach_body_darccos(module: &mut Module) {
    let fid = match module.lookup_runtime("darccos") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("darccos", sig, Visibility::Public);
    let x = b.param(0);

    let acos_val = b.call("builtin.acos_f64", &[x], Type::Float(64));
    let factor = b.const_float(180.0 / PI);
    let result = b.mul(acos_val, factor);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// darctan(x: f64) -> f64  =  atan(x) * 180/π
// ---------------------------------------------------------------------------

fn attach_body_darctan(module: &mut Module) {
    let fid = match module.lookup_runtime("darctan") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("darctan", sig, Visibility::Public);
    let x = b.param(0);

    let atan_val = b.call("builtin.atan_f64", &[x], Type::Float(64));
    let factor = b.const_float(180.0 / PI);
    let result = b.mul(atan_val, factor);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// darctan2(y: f64, x: f64) -> f64  =  atan2(y, x) * 180/π
// ---------------------------------------------------------------------------

fn attach_body_darctan2(module: &mut Module) {
    let fid = match module.lookup_runtime("darctan2") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("darctan2", sig, Visibility::Public);
    let y = b.param(0);
    let x = b.param(1);

    let atan2_val = b.call("builtin.atan2_f64", &[y, x], Type::Float(64));
    let factor = b.const_float(180.0 / PI);
    let result = b.mul(atan2_val, factor);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// arctan2(y: f64, x: f64) -> f64  =  atan2(y, x)  [result in radians]
// ---------------------------------------------------------------------------

fn attach_body_arctan2(module: &mut Module) {
    let fid = match module.lookup_runtime("arctan2") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("arctan2", sig, Visibility::Public);
    let y = b.param(0);
    let x = b.param(1);

    let result = b.call("builtin.atan2_f64", &[y, x], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
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
    let fid = match module.lookup_runtime("point_direction") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
        ],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("point_direction", sig, Visibility::Public);
    let x1 = b.param(0);
    let y1 = b.param(1);
    let x2 = b.param(2);
    let y2 = b.param(3);

    let dy = b.sub(y1, y2);
    let dx = b.sub(x2, x1);
    let atan2_val = b.call("builtin.atan2_f64", &[dy, dx], Type::Float(64));
    let factor = b.const_float(180.0 / PI);
    let result = b.mul(atan2_val, factor);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// sqr(x: f64) -> f64  =  x * x
// ---------------------------------------------------------------------------

fn attach_body_sqr(module: &mut Module) {
    let fid = match module.lookup_runtime("sqr") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("sqr", sig, Visibility::Public);
    let x = b.param(0);

    let result = b.mul(x, x);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// power(base: f64, exp: f64) -> f64  =  pow(base, exp)
// ---------------------------------------------------------------------------

fn attach_body_power(module: &mut Module) {
    let fid = match module.lookup_runtime("power") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("power", sig, Visibility::Public);
    let base = b.param(0);
    let exp = b.param(1);

    let result = b.call("builtin.pow_f64", &[base, exp], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// logn(n: f64, val: f64) -> f64  =  ln(val) / ln(n)
// ---------------------------------------------------------------------------

fn attach_body_logn(module: &mut Module) {
    let fid = match module.lookup_runtime("logn") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("logn", sig, Visibility::Public);
    let n = b.param(0);
    let val = b.param(1);

    let ln_val = b.call("builtin.ln_f64", &[val], Type::Float(64));
    let ln_n = b.call("builtin.ln_f64", &[n], Type::Float(64));
    let result = b.div(ln_val, ln_n);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// log2(x: f64) -> f64  =  log2(x)
// ---------------------------------------------------------------------------

fn attach_body_log2(module: &mut Module) {
    let fid = match module.lookup_runtime("log2") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("log2", sig, Visibility::Public);
    let x = b.param(0);

    let result = b.call("builtin.log2_f64", &[x], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// log10(x: f64) -> f64  =  log10(x)
// ---------------------------------------------------------------------------

fn attach_body_log10(module: &mut Module) {
    let fid = match module.lookup_runtime("log10") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("log10", sig, Visibility::Public);
    let x = b.param(0);

    let result = b.call("builtin.log10_f64", &[x], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// exp(x: f64) -> f64  =  e^x
// ---------------------------------------------------------------------------

fn attach_body_exp(module: &mut Module) {
    let fid = match module.lookup_runtime("exp") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("exp", sig, Visibility::Public);
    let x = b.param(0);

    let result = b.call("builtin.exp_f64", &[x], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// clamp(val: f64, min: f64, max: f64) -> f64  =  min_f64(max_f64(val, min), max)
// ---------------------------------------------------------------------------

fn attach_body_clamp(module: &mut Module) {
    let fid = match module.lookup_runtime("clamp") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("clamp", sig, Visibility::Public);
    let val = b.param(0);
    let lo = b.param(1);
    let hi = b.param(2);

    let clamped_lo = b.call("builtin.max_f64", &[val, lo], Type::Float(64));
    let result = b.call("builtin.min_f64", &[clamped_lo, hi], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// lerp(a: f64, b: f64, amt: f64) -> f64  =  a * (1 - amt) + b * amt
// ---------------------------------------------------------------------------

fn attach_body_lerp(module: &mut Module) {
    let fid = match module.lookup_runtime("lerp") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("lerp", sig, Visibility::Public);
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

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// abs(x: f64) -> f64  =  abs(x)
// ---------------------------------------------------------------------------

fn attach_body_abs(module: &mut Module) {
    let fid = match module.lookup_runtime("abs") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("abs", sig, Visibility::Public);
    let x = b.param(0);

    let result = b.call("builtin.abs_f64", &[x], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// floor(x: f64) -> f64  =  floor(x)
// ---------------------------------------------------------------------------

fn attach_body_floor(module: &mut Module) {
    let fid = match module.lookup_runtime("floor") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("floor", sig, Visibility::Public);
    let x = b.param(0);

    let result = b.call("builtin.floor_f64", &[x], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// ceil(x: f64) -> f64  =  ceil(x)
// ---------------------------------------------------------------------------

fn attach_body_ceil(module: &mut Module) {
    let fid = match module.lookup_runtime("ceil") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("ceil", sig, Visibility::Public);
    let x = b.param(0);

    let result = b.call("builtin.ceil_f64", &[x], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// round(x: f64) -> f64  =  round(x)
//
// GML round uses round-half-away-from-zero, same as Math.round for positive
// values and mirrored for negative.  builtin.round_f64 maps to Math.round.
// ---------------------------------------------------------------------------

fn attach_body_round(module: &mut Module) {
    let fid = match module.lookup_runtime("round") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("round", sig, Visibility::Public);
    let x = b.param(0);

    let result = b.call("builtin.round_f64", &[x], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// sign(x: f64) -> f64  =  sign(x)  [returns -1, 0, or 1]
// ---------------------------------------------------------------------------

fn attach_body_sign(module: &mut Module) {
    let fid = match module.lookup_runtime("sign") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("sign", sig, Visibility::Public);
    let x = b.param(0);

    let result = b.call("builtin.sign_f64", &[x], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// sqrt(x: f64) -> f64  =  sqrt(x)
// ---------------------------------------------------------------------------

fn attach_body_sqrt(module: &mut Module) {
    let fid = match module.lookup_runtime("sqrt") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("sqrt", sig, Visibility::Public);
    let x = b.param(0);

    let result = b.call("builtin.sqrt_f64", &[x], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// arctan(x: f64) -> f64  =  atan(x)  [result in radians]
// ---------------------------------------------------------------------------

fn attach_body_arctan(module: &mut Module) {
    let fid = match module.lookup_runtime("arctan") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("arctan", sig, Visibility::Public);
    let x = b.param(0);

    let result = b.call("builtin.atan_f64", &[x], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// frac(x: f64) -> f64  =  x - trunc(x)
//
// Returns the fractional part of x (the digits after the decimal point).
// For negative values, e.g. frac(-3.7) = -3.7 - (-3.0) = -0.7.
// ---------------------------------------------------------------------------

fn attach_body_frac(module: &mut Module) {
    let fid = match module.lookup_runtime("frac") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("frac", sig, Visibility::Public);
    let x = b.param(0);

    let trunc_val = b.call("builtin.trunc_f64", &[x], Type::Float(64));
    let result = b.sub(x, trunc_val);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// dot_product(x1, y1, x2, y2: f64) -> f64  =  x1*x2 + y1*y2
// ---------------------------------------------------------------------------

fn attach_body_dot_product(module: &mut Module) {
    let fid = match module.lookup_runtime("dot_product") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
        ],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("dot_product", sig, Visibility::Public);
    let x1 = b.param(0);
    let y1 = b.param(1);
    let x2 = b.param(2);
    let y2 = b.param(3);

    let x_term = b.mul(x1, x2);
    let y_term = b.mul(y1, y2);
    let result = b.add(x_term, y_term);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// dot_product_3d(x1, y1, z1, x2, y2, z2: f64) -> f64  =  x1*x2 + y1*y2 + z1*z2
// ---------------------------------------------------------------------------

fn attach_body_dot_product_3d(module: &mut Module) {
    let fid = match module.lookup_runtime("dot_product_3d") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
            Type::Float(64),
        ],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("dot_product_3d", sig, Visibility::Public);
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

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// color_get_red(color: f64) -> f64  =  color & 0xFF
//
// GML colors use BGR byte order; red is in the low byte.
// ---------------------------------------------------------------------------

fn attach_body_color_get_red(module: &mut Module) {
    let fid = match module.lookup_runtime("color_get_red") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("color_get_red", sig, Visibility::Public);
    let color = b.param(0);

    let mask = b.const_float(255.0);
    let result = b.bit_and(color, mask);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// color_get_green(color: f64) -> f64  =  (color >> 8) & 0xFF
//
// Green occupies the middle byte.
// ---------------------------------------------------------------------------

fn attach_body_color_get_green(module: &mut Module) {
    let fid = match module.lookup_runtime("color_get_green") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("color_get_green", sig, Visibility::Public);
    let color = b.param(0);

    let shift = b.const_float(8.0);
    let shifted = b.shr(color, shift);
    let mask = b.const_float(255.0);
    let result = b.bit_and(shifted, mask);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// color_get_blue(color: f64) -> f64  =  color >> 16
//
// Blue occupies the high byte; no mask needed after shifting.
// ---------------------------------------------------------------------------

fn attach_body_color_get_blue(module: &mut Module) {
    let fid = match module.lookup_runtime("color_get_blue") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("color_get_blue", sig, Visibility::Public);
    let color = b.param(0);

    let shift = b.const_float(16.0);
    let result = b.shr(color, shift);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// make_color_rgb(r: f64, g: f64, b: f64) -> f64  =  (b << 16) | (g << 8) | r
//
// Packs RGB components into a GML BGR color value.
// ---------------------------------------------------------------------------

fn attach_body_make_color_rgb(module: &mut Module) {
    let fid = match module.lookup_runtime("make_color_rgb") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("make_color_rgb", sig, Visibility::Public);
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

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// colour_get_red(color: f64) -> f64  =  color & 0xFF
// ---------------------------------------------------------------------------

fn attach_body_colour_get_red(module: &mut Module) {
    let fid = match module.lookup_runtime("colour_get_red") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("colour_get_red", sig, Visibility::Public);
    let color = b.param(0);

    let mask = b.const_float(255.0);
    let result = b.bit_and(color, mask);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// colour_get_green(color: f64) -> f64  =  (color >> 8) & 0xFF
// ---------------------------------------------------------------------------

fn attach_body_colour_get_green(module: &mut Module) {
    let fid = match module.lookup_runtime("colour_get_green") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("colour_get_green", sig, Visibility::Public);
    let color = b.param(0);

    let shift = b.const_float(8.0);
    let shifted = b.shr(color, shift);
    let mask = b.const_float(255.0);
    let result = b.bit_and(shifted, mask);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// colour_get_blue(color: f64) -> f64  =  color >> 16
// ---------------------------------------------------------------------------

fn attach_body_colour_get_blue(module: &mut Module) {
    let fid = match module.lookup_runtime("colour_get_blue") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("colour_get_blue", sig, Visibility::Public);
    let color = b.param(0);

    let shift = b.const_float(16.0);
    let result = b.shr(color, shift);
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// make_colour_rgb(r: f64, g: f64, b: f64) -> f64  =  (b << 16) | (g << 8) | r
// ---------------------------------------------------------------------------

fn attach_body_make_colour_rgb(module: &mut Module) {
    let fid = match module.lookup_runtime("make_colour_rgb") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("make_colour_rgb", sig, Visibility::Public);
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

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
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
    let fid = match module.lookup_runtime("merge_color") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("merge_color", sig, Visibility::Public);
    let col1 = b.param(0);
    let col2 = b.param(1);
    let amt = b.param(2);

    let one = b.const_float(1.0);
    let one_minus_amt = b.sub(one, amt);

    // Red channel
    let r1 = b.call("color_get_red", &[col1], Type::Float(64));
    let r2 = b.call("color_get_red", &[col2], Type::Float(64));
    let r1_part = b.mul(r1, one_minus_amt);
    let r2_part = b.mul(r2, amt);
    let r_blend = b.add(r1_part, r2_part);
    let r_out = b.call("builtin.round_f64", &[r_blend], Type::Float(64));

    // Green channel
    let g1 = b.call("color_get_green", &[col1], Type::Float(64));
    let g2 = b.call("color_get_green", &[col2], Type::Float(64));
    let g1_part = b.mul(g1, one_minus_amt);
    let g2_part = b.mul(g2, amt);
    let g_blend = b.add(g1_part, g2_part);
    let g_out = b.call("builtin.round_f64", &[g_blend], Type::Float(64));

    // Blue channel
    let b1 = b.call("color_get_blue", &[col1], Type::Float(64));
    let b2 = b.call("color_get_blue", &[col2], Type::Float(64));
    let b1_part = b.mul(b1, one_minus_amt);
    let b2_part = b.mul(b2, amt);
    let b_blend = b.add(b1_part, b2_part);
    let bv_out = b.call("builtin.round_f64", &[b_blend], Type::Float(64));

    let result = b.call("make_color_rgb", &[r_out, g_out, bv_out], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// merge_colour — alias for merge_color
// ---------------------------------------------------------------------------

fn attach_body_merge_colour(module: &mut Module) {
    let fid = match module.lookup_runtime("merge_colour") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("merge_colour", sig, Visibility::Public);
    let col1 = b.param(0);
    let col2 = b.param(1);
    let amt = b.param(2);

    let one = b.const_float(1.0);
    let one_minus_amt = b.sub(one, amt);

    let r1 = b.call("color_get_red", &[col1], Type::Float(64));
    let r2 = b.call("color_get_red", &[col2], Type::Float(64));
    let r1_part = b.mul(r1, one_minus_amt);
    let r2_part = b.mul(r2, amt);
    let r_blend = b.add(r1_part, r2_part);
    let r_out = b.call("builtin.round_f64", &[r_blend], Type::Float(64));

    let g1 = b.call("color_get_green", &[col1], Type::Float(64));
    let g2 = b.call("color_get_green", &[col2], Type::Float(64));
    let g1_part = b.mul(g1, one_minus_amt);
    let g2_part = b.mul(g2, amt);
    let g_blend = b.add(g1_part, g2_part);
    let g_out = b.call("builtin.round_f64", &[g_blend], Type::Float(64));

    let b1 = b.call("color_get_blue", &[col1], Type::Float(64));
    let b2 = b.call("color_get_blue", &[col2], Type::Float(64));
    let b1_part = b.mul(b1, one_minus_amt);
    let b2_part = b.mul(b2, amt);
    let b_blend = b.add(b1_part, b2_part);
    let bv_out = b.call("builtin.round_f64", &[b_blend], Type::Float(64));

    let result = b.call("make_color_rgb", &[r_out, g_out, bv_out], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
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
    let fid = match module.lookup_runtime("color_get_value") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("color_get_value", sig, Visibility::Public);
    let color = b.param(0);

    let c255 = b.const_float(255.0);

    let r_raw = b.call("color_get_red", &[color], Type::Float(64));
    let g_raw = b.call("color_get_green", &[color], Type::Float(64));
    let bv_raw = b.call("color_get_blue", &[color], Type::Float(64));
    let r = b.div(r_raw, c255);
    let g = b.div(g_raw, c255);
    let bv = b.div(bv_raw, c255);

    let max_rg = b.call("builtin.max_f64", &[r, g], Type::Float(64));
    let max_rgb = b.call("builtin.max_f64", &[max_rg, bv], Type::Float(64));

    let scaled = b.mul(max_rgb, c255);
    let result = b.call("builtin.round_f64", &[scaled], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// colour_get_value — alias for color_get_value
// ---------------------------------------------------------------------------

fn attach_body_colour_get_value(module: &mut Module) {
    let fid = match module.lookup_runtime("colour_get_value") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("colour_get_value", sig, Visibility::Public);
    let color = b.param(0);

    let c255 = b.const_float(255.0);

    let r_raw = b.call("color_get_red", &[color], Type::Float(64));
    let g_raw = b.call("color_get_green", &[color], Type::Float(64));
    let bv_raw = b.call("color_get_blue", &[color], Type::Float(64));
    let r = b.div(r_raw, c255);
    let g = b.div(g_raw, c255);
    let bv = b.div(bv_raw, c255);

    let max_rg = b.call("builtin.max_f64", &[r, g], Type::Float(64));
    let max_rgb = b.call("builtin.max_f64", &[max_rg, bv], Type::Float(64));

    let scaled = b.mul(max_rgb, c255);
    let result = b.call("builtin.round_f64", &[scaled], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
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
    let fid = match module.lookup_runtime("color_get_saturation") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("color_get_saturation", sig, Visibility::Public);
    let color = b.param(0);

    let c255 = b.const_float(255.0);
    let zero = b.const_float(0.0);

    let r_raw = b.call("color_get_red", &[color], Type::Float(64));
    let g_raw = b.call("color_get_green", &[color], Type::Float(64));
    let bv_raw = b.call("color_get_blue", &[color], Type::Float(64));
    let r = b.div(r_raw, c255);
    let g = b.div(g_raw, c255);
    let bv = b.div(bv_raw, c255);

    let max_rg = b.call("builtin.max_f64", &[r, g], Type::Float(64));
    let max_rgb = b.call("builtin.max_f64", &[max_rg, bv], Type::Float(64));
    let min_rg = b.call("builtin.min_f64", &[r, g], Type::Float(64));
    let min_rgb = b.call("builtin.min_f64", &[min_rg, bv], Type::Float(64));

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
    let result = b.call("builtin.round_f64", &[scaled], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// colour_get_saturation — alias for color_get_saturation
// ---------------------------------------------------------------------------

fn attach_body_colour_get_saturation(module: &mut Module) {
    let fid = match module.lookup_runtime("colour_get_saturation") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("colour_get_saturation", sig, Visibility::Public);
    let color = b.param(0);

    let c255 = b.const_float(255.0);
    let zero = b.const_float(0.0);

    let r_raw = b.call("color_get_red", &[color], Type::Float(64));
    let g_raw = b.call("color_get_green", &[color], Type::Float(64));
    let bv_raw = b.call("color_get_blue", &[color], Type::Float(64));
    let r = b.div(r_raw, c255);
    let g = b.div(g_raw, c255);
    let bv = b.div(bv_raw, c255);

    let max_rg = b.call("builtin.max_f64", &[r, g], Type::Float(64));
    let max_rgb = b.call("builtin.max_f64", &[max_rg, bv], Type::Float(64));
    let min_rg = b.call("builtin.min_f64", &[r, g], Type::Float(64));
    let min_rgb = b.call("builtin.min_f64", &[min_rg, bv], Type::Float(64));

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
    let result = b.call("builtin.round_f64", &[scaled], Type::Float(64));
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
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
    let fid = match module.lookup_runtime("color_get_hue") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("color_get_hue", sig, Visibility::Public);
    let color = b.param(0);

    let c255 = b.const_float(255.0);
    let zero = b.const_float(0.0);

    // Extract r, g, b as fractions in [0, 1].
    let r_raw = b.call("color_get_red", &[color], Type::Float(64));
    let g_raw = b.call("color_get_green", &[color], Type::Float(64));
    let bv_raw = b.call("color_get_blue", &[color], Type::Float(64));
    let r = b.div(r_raw, c255);
    let g = b.div(g_raw, c255);
    let bv = b.div(bv_raw, c255);

    let max_rg = b.call("builtin.max_f64", &[r, g], Type::Float(64));
    let max_rgb = b.call("builtin.max_f64", &[max_rg, bv], Type::Float(64));
    let min_rg = b.call("builtin.min_f64", &[r, g], Type::Float(64));
    let min_rgb = b.call("builtin.min_f64", &[min_rg, bv], Type::Float(64));
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
    let r_is_max = b.call("builtin.and_bool", &[r_ge_g, r_ge_b], Type::Bool);

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
    let h_rounded = b.call("builtin.round_f64", &[h_scaled], Type::Float(64));

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

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// colour_get_hue — alias for color_get_hue
// ---------------------------------------------------------------------------

fn attach_body_colour_get_hue(module: &mut Module) {
    let fid = match module.lookup_runtime("colour_get_hue") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("colour_get_hue", sig, Visibility::Public);
    let color = b.param(0);

    let c255 = b.const_float(255.0);
    let zero = b.const_float(0.0);

    let r_raw = b.call("color_get_red", &[color], Type::Float(64));
    let g_raw = b.call("color_get_green", &[color], Type::Float(64));
    let bv_raw = b.call("color_get_blue", &[color], Type::Float(64));
    let r = b.div(r_raw, c255);
    let g = b.div(g_raw, c255);
    let bv = b.div(bv_raw, c255);

    let max_rg = b.call("builtin.max_f64", &[r, g], Type::Float(64));
    let max_rgb = b.call("builtin.max_f64", &[max_rg, bv], Type::Float(64));
    let min_rg = b.call("builtin.min_f64", &[r, g], Type::Float(64));
    let min_rgb = b.call("builtin.min_f64", &[min_rg, bv], Type::Float(64));
    let d = b.sub(max_rgb, min_rgb);

    let d_le_zero = b.cmp(CmpKind::Le, d, zero);
    let ret_zero_block = b.create_block();
    let branch_r_check = b.create_block();
    b.br_if(d_le_zero, ret_zero_block, &[], branch_r_check, &[]);

    b.switch_to_block(ret_zero_block);
    b.ret(Some(zero));

    b.switch_to_block(branch_r_check);
    let r_ge_g = b.cmp(CmpKind::Ge, r, g);
    let r_ge_b = b.cmp(CmpKind::Ge, r, bv);
    let r_is_max = b.call("builtin.and_bool", &[r_ge_g, r_ge_b], Type::Bool);

    let (merge_block, h_params) = b.create_block_with_params(&[Type::Float(64)]);
    let block_r = b.create_block();
    let block_not_r = b.create_block();
    b.br_if(r_is_max, block_r, &[], block_not_r, &[]);

    b.switch_to_block(block_r);
    let c6 = b.const_float(6.0);
    let g_minus_b = b.sub(g, bv);
    let h_r_raw = b.div(g_minus_b, d);
    let h_r = b.rem(h_r_raw, c6);
    b.br(merge_block, &[h_r]);

    b.switch_to_block(block_not_r);
    let g_ge_b = b.cmp(CmpKind::Ge, g, bv);
    let block_g = b.create_block();
    let block_bv = b.create_block();
    b.br_if(g_ge_b, block_g, &[], block_bv, &[]);

    b.switch_to_block(block_g);
    let two = b.const_float(2.0);
    let bv_minus_r = b.sub(bv, r);
    let h_g_div = b.div(bv_minus_r, d);
    let h_g = b.add(h_g_div, two);
    b.br(merge_block, &[h_g]);

    b.switch_to_block(block_bv);
    let four = b.const_float(4.0);
    let r_minus_g = b.sub(r, g);
    let h_bv_div = b.div(r_minus_g, d);
    let h_bv = b.add(h_bv_div, four);
    b.br(merge_block, &[h_bv]);

    b.switch_to_block(merge_block);
    let h_raw = h_params[0];

    let c255_over_6 = b.const_float(255.0 / 6.0);
    let h_scaled = b.mul(h_raw, c255_over_6);
    let h_rounded = b.call("builtin.round_f64", &[h_scaled], Type::Float(64));

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

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
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
    let fid = match module.lookup_runtime("make_color_hsv") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("make_color_hsv", sig, Visibility::Public);
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
    let abs_val = b.call("builtin.abs_f64", &[hf_mod2_m1], Type::Float(64));
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
    let r_final = b.call("builtin.round_f64", &[r_scaled], Type::Float(64));
    let g_plus_m = b.add(g_out, m);
    let g_scaled = b.mul(g_plus_m, c255);
    let g_final = b.call("builtin.round_f64", &[g_scaled], Type::Float(64));
    let bv_plus_m = b.add(bv_out, m);
    let bv_scaled = b.mul(bv_plus_m, c255);
    let b_final = b.call("builtin.round_f64", &[bv_scaled], Type::Float(64));

    let result = b.call(
        "make_color_rgb",
        &[r_final, g_final, b_final],
        Type::Float(64),
    );
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}

// ---------------------------------------------------------------------------
// make_colour_hsv — alias for make_color_hsv
// ---------------------------------------------------------------------------

fn attach_body_make_colour_hsv(module: &mut Module) {
    let fid = match module.lookup_runtime("make_colour_hsv") {
        Some(id) => id,
        None => return,
    };

    let sig = FunctionSig {
        params: vec![Type::Float(64), Type::Float(64), Type::Float(64)],
        return_ty: Type::Float(64),
        defaults: vec![],
        has_rest_param: false,
    };

    let mut b = FunctionBuilder::new("make_colour_hsv", sig, Visibility::Public);
    let h = b.param(0);
    let s = b.param(1);
    let v = b.param(2);

    let c255 = b.const_float(255.0);
    let c6 = b.const_float(6.0);

    let h_div = b.div(h, c255);
    let hf = b.mul(h_div, c6);
    let sf = b.div(s, c255);
    let vf = b.div(v, c255);

    let cv = b.mul(vf, sf);
    let c2_hsv = b.const_float(2.0);
    let hf_mod2 = b.rem(hf, c2_hsv);
    let c1_hsv = b.const_float(1.0);
    let hf_mod2_m1 = b.sub(hf_mod2, c1_hsv);
    let abs_val = b.call("builtin.abs_f64", &[hf_mod2_m1], Type::Float(64));
    let c1_hsv2 = b.const_float(1.0);
    let one_minus_abs = b.sub(c1_hsv2, abs_val);
    let x = b.mul(cv, one_minus_abs);
    let m = b.sub(vf, cv);

    let (merge_block, rgb_params) =
        b.create_block_with_params(&[Type::Float(64), Type::Float(64), Type::Float(64)]);

    let c1 = b.const_float(1.0);
    let c2 = b.const_float(2.0);
    let c3 = b.const_float(3.0);
    let c4 = b.const_float(4.0);
    let c5 = b.const_float(5.0);
    let zero = b.const_float(0.0);

    let hf_lt1 = b.cmp(CmpKind::Lt, hf, c1);
    let blk0 = b.create_block();
    let blk_ge1 = b.create_block();
    b.br_if(hf_lt1, blk0, &[], blk_ge1, &[]);

    b.switch_to_block(blk0);
    b.br(merge_block, &[cv, x, zero]);

    b.switch_to_block(blk_ge1);
    let hf_lt2 = b.cmp(CmpKind::Lt, hf, c2);
    let blk1 = b.create_block();
    let blk_ge2 = b.create_block();
    b.br_if(hf_lt2, blk1, &[], blk_ge2, &[]);

    b.switch_to_block(blk1);
    b.br(merge_block, &[x, cv, zero]);

    b.switch_to_block(blk_ge2);
    let hf_lt3 = b.cmp(CmpKind::Lt, hf, c3);
    let blk2 = b.create_block();
    let blk_ge3 = b.create_block();
    b.br_if(hf_lt3, blk2, &[], blk_ge3, &[]);

    b.switch_to_block(blk2);
    b.br(merge_block, &[zero, cv, x]);

    b.switch_to_block(blk_ge3);
    let hf_lt4 = b.cmp(CmpKind::Lt, hf, c4);
    let blk3 = b.create_block();
    let blk_ge4 = b.create_block();
    b.br_if(hf_lt4, blk3, &[], blk_ge4, &[]);

    b.switch_to_block(blk3);
    b.br(merge_block, &[zero, x, cv]);

    b.switch_to_block(blk_ge4);
    let hf_lt5 = b.cmp(CmpKind::Lt, hf, c5);
    let blk4 = b.create_block();
    let blk5 = b.create_block();
    b.br_if(hf_lt5, blk4, &[], blk5, &[]);

    b.switch_to_block(blk4);
    b.br(merge_block, &[x, zero, cv]);

    b.switch_to_block(blk5);
    b.br(merge_block, &[cv, zero, x]);

    b.switch_to_block(merge_block);
    let r_out = rgb_params[0];
    let g_out = rgb_params[1];
    let bv_out = rgb_params[2];

    let r_plus_m = b.add(r_out, m);
    let r_scaled = b.mul(r_plus_m, c255);
    let r_final = b.call("builtin.round_f64", &[r_scaled], Type::Float(64));
    let g_plus_m = b.add(g_out, m);
    let g_scaled = b.mul(g_plus_m, c255);
    let g_final = b.call("builtin.round_f64", &[g_scaled], Type::Float(64));
    let bv_plus_m = b.add(bv_out, m);
    let bv_scaled = b.mul(bv_plus_m, c255);
    let b_final = b.call("builtin.round_f64", &[bv_scaled], Type::Float(64));

    let result = b.call(
        "make_color_rgb",
        &[r_final, g_final, b_final],
        Type::Float(64),
    );
    b.ret(Some(result));

    let built = b.build();
    let stub = &mut module.functions[fid];
    stub.blocks = built.blocks;
    stub.insts = built.insts;
    stub.value_types = built.value_types;
    stub.entry = built.entry;
}
