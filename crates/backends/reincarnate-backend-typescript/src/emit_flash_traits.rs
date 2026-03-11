//! Flash AVM2 class emission helpers.
//!
//! Contains reflection metadata (`registerClassTraits`), qualified-name headers,
//! constructor shim parameters, and forwarding-setter detection — all specific
//! to the Flash/AS3 getter/setter naming convention (`get_`/`set_` prefixes).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;

use reincarnate_core::ir::{FuncId, MethodKind, Module, Type};

use crate::emit::{qualified_class_name, sanitize_ident, ClassGroup};

/// Map an IR `Type` to the AS3-style type name used in describeType output.
pub(crate) fn as3_type_name(ty: &Type) -> String {
    match ty {
        Type::Void => "void".into(),
        Type::Bool => "Boolean".into(),
        Type::Int(_) => "int".into(),
        Type::UInt(_) => "uint".into(),
        Type::Float(_) => "Number".into(),
        Type::String => "String".into(),
        Type::Array(_) => "Array".into(),
        Type::Map(_, _) => "Object".into(),
        Type::Struct(name) | Type::Enum(name) | Type::ClassRef(name) => {
            name.rsplit("::").next().unwrap_or(name).into()
        }
        _ => "*".into(),
    }
}

/// Emit a `registerClassTraits(ClassName, [...instance], [...static])` call,
/// with each trait object on its own indented line for readable git diffs.
pub(crate) fn emit_class_registration(
    group: &ClassGroup,
    module: &Module,
    class_names: &HashMap<String, String>,
    out: &mut String,
) {
    let qualified = qualified_class_name(&group.class_def);
    let class_name = class_names
        .get(&qualified)
        .cloned()
        .unwrap_or_else(|| sanitize_ident(&group.class_def.name));

    // Collect instance traits: fields from struct_def + instance methods/getters/setters
    let mut instance_traits: Vec<String> = Vec::new();
    for (name, ty, _) in &group.struct_def.fields {
        let type_name = as3_type_name(ty);
        instance_traits.push(format!(
            "  {{ name: \"{name}\", kind: \"variable\", type: \"{type_name}\" }}"
        ));
    }

    // Collect static traits: fields from class_def + static methods
    let mut static_traits: Vec<String> = Vec::new();
    for f in &group.class_def.static_fields {
        let type_name = as3_type_name(&f.ty);
        static_traits.push(format!(
            "  {{ name: \"{}\", kind: \"variable\", type: \"{type_name}\" }}",
            f.name
        ));
    }

    // Track getter/setter pairs to coalesce into accessors
    let mut instance_accessors: BTreeMap<String, (bool, bool)> = BTreeMap::new();
    for &fid in &group.methods {
        let func = &module.functions[fid];
        // Strip class prefix: "Enum::toString" → "toString"
        let short = func.name.rsplit("::").next().unwrap_or(&func.name);
        match func.method_kind {
            MethodKind::Constructor
            | MethodKind::Free
            | MethodKind::Closure
            | MethodKind::StaticInit => {}
            MethodKind::Instance => {
                instance_traits.push(format!("  {{ name: \"{short}\", kind: \"method\" }}"));
            }
            MethodKind::Static => {
                static_traits.push(format!("  {{ name: \"{short}\", kind: \"method\" }}"));
            }
            MethodKind::Getter => {
                // Strip get_ prefix to match AS3 accessor names
                let acc_name = short.strip_prefix("get_").unwrap_or(short);
                let entry = instance_accessors
                    .entry(acc_name.to_string())
                    .or_insert((false, false));
                entry.0 = true;
            }
            MethodKind::Setter => {
                let acc_name = short.strip_prefix("set_").unwrap_or(short);
                let entry = instance_accessors
                    .entry(acc_name.to_string())
                    .or_insert((false, false));
                entry.1 = true;
            }
        }
    }

    // Emit coalesced accessors
    for (name, (has_get, has_set)) in &instance_accessors {
        let access = match (has_get, has_set) {
            (true, true) => "readwrite",
            (true, false) => "readonly",
            (false, true) => "writeonly",
            _ => "readwrite",
        };
        instance_traits.push(format!(
            "  {{ name: \"{name}\", kind: \"accessor\", access: \"{access}\" }}"
        ));
    }

    // Format as multi-line for readable diffs.
    let instance_body = if instance_traits.is_empty() {
        String::new()
    } else {
        format!("\n{},\n", instance_traits.join(",\n"))
    };
    let static_body = if static_traits.is_empty() {
        String::new()
    } else {
        format!("\n{},\n", static_traits.join(",\n"))
    };

    let _ = writeln!(
        out,
        "registerClassTraits({class_name}, [{instance_body}], [{static_body}]);\n"
    );
}

/// Emit the `static [QN_KEY]: string = "pkg::ClassName";` line at the top of
/// every Flash class body.  `parent_in_module` controls whether `override` is
/// needed (in-module parent already declares `[QN_KEY]`; external runtime
/// parents do not).
pub(crate) fn emit_flash_class_header(qualified: &str, parent_in_module: bool) -> String {
    let ov = if parent_in_module { " override" } else { "" };
    format!("  static{ov} [QN_KEY]: string = \"{qualified}\";\n")
}

/// Returns the `_shims: FlashShims` constructor parameter string for Flash
/// class constructors, or `None` for non-constructor methods.
/// `suppress_super` / `parent_is_runtime`: if either is true the parameter is
/// `readonly` (base-class or runtime-parent constructor stores it as a field).
pub(crate) fn flash_ctor_shims_param(suppress_super: bool, parent_is_runtime: bool) -> String {
    if suppress_super || parent_is_runtime {
        "readonly _shims: FlashShims".to_string()
    } else {
        "_shims: FlashShims".to_string()
    }
}

/// Detect getter overrides without matching setter overrides in Flash classes.
///
/// When a subclass overrides a getter but not the corresponding setter, and the
/// parent has a setter for that property, TypeScript treats the property as
/// read-only in the subclass (TS2540). This returns `(property_name, type_str)`
/// pairs for which a forwarding `override set prop(value: T) { super.prop = value; }`
/// should be emitted.
///
/// Flash-specific: relies on the `get_`/`set_` naming convention for AS3 accessors.
pub(crate) fn flash_forwarding_setters(
    module: &Module,
    sorted_methods: &[FuncId],
    parent_method_names: &HashSet<String>,
) -> Vec<(String, String)> {
    let own_getter_props: HashSet<String> = sorted_methods
        .iter()
        .filter_map(|&fid| {
            let f = &module.functions[fid];
            if matches!(f.method_kind, MethodKind::Getter) {
                let short = f.name.rsplit("::").next().unwrap_or(&f.name);
                short.strip_prefix("get_").map(|p| p.to_string())
            } else {
                None
            }
        })
        .collect();
    let own_setter_props: HashSet<String> = sorted_methods
        .iter()
        .filter_map(|&fid| {
            let f = &module.functions[fid];
            if matches!(f.method_kind, MethodKind::Setter) {
                let short = f.name.rsplit("::").next().unwrap_or(&f.name);
                short.strip_prefix("set_").map(|p| p.to_string())
            } else {
                None
            }
        })
        .collect();
    own_getter_props
        .into_iter()
        .filter(|prop| {
            !own_setter_props.contains(prop.as_str())
                && parent_method_names.contains(&format!("set_{prop}"))
        })
        .map(|prop| {
            // Use the getter's return type as the setter parameter type.
            let ty = sorted_methods
                .iter()
                .find_map(|&fid| {
                    let f = &module.functions[fid];
                    if matches!(f.method_kind, MethodKind::Getter) {
                        let short = f.name.rsplit("::").next().unwrap_or(&f.name);
                        if short.strip_prefix("get_") == Some(prop.as_str()) {
                            return Some(crate::types::ts_type(&f.sig.return_ty));
                        }
                    }
                    None
                })
                .unwrap_or_else(|| "any".to_string());
            (prop, ty)
        })
        .collect()
}
