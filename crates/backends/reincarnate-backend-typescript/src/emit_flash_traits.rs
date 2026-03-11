//! Flash AVM2 reflection metadata emission.
//!
//! Emits `registerClassTraits(ClassName, [...instance], [...static])` calls
//! that feed the AS3 `describeType`-compatible reflection layer at runtime.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use reincarnate_core::ir::{MethodKind, Module, Type};

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
    for (name, ty, _, _) in &group.class_def.static_fields {
        let type_name = as3_type_name(ty);
        static_traits.push(format!(
            "  {{ name: \"{name}\", kind: \"variable\", type: \"{type_name}\" }}"
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
