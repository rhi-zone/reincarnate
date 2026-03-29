use std::f64::consts::PI;

use reincarnate_core::ir::builder::FunctionBuilder;
use reincarnate_core::ir::func::Visibility;
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
