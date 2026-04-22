use reincarnate_core::ir::value::Constant;

/// Map a GML builtin variable name to its IR constant value, if it is a constant.
/// Returns None for mutable builtins (x, y, direction, etc.).
pub(super) fn gml_builtin_constant(name: &str) -> Option<Constant> {
    match name {
        "undefined" => Some(Constant::Null),
        "noone" => Some(Constant::Float(-4.0)),
        "pi" => Some(Constant::Float(std::f64::consts::PI)),
        "inf" | "infinity" => Some(Constant::Float(f64::INFINITY)),
        "NaN" => Some(Constant::Float(f64::NAN)),
        "pointer_null" => Some(Constant::Int(0)),
        "pointer_invalid" => Some(Constant::Int(-1)),
        _ => None,
    }
}
