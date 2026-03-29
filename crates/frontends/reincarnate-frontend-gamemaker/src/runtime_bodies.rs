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
