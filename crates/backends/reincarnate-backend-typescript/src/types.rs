use std::collections::HashMap;

use reincarnate_core::ir::Type;

use crate::emit::sanitize_ident;

/// Map an IR [`Type`] to its TypeScript representation.
pub fn ts_type(ty: &Type) -> String {
    match ty {
        Type::Void => "void".into(),
        Type::Bool => "boolean".into(),
        Type::Int(_) | Type::UInt(_) | Type::Float(_) => "number".into(),
        Type::String => "string".into(),
        Type::Array(elem) => format!("{}[]", ts_type_paren(elem)),
        Type::Map(k, v) => {
            // Map keys should be `unknown` rather than `any` — `any` disables
            // type checking on lookups while `unknown` forces explicit narrowing.
            let key = if matches!(k.as_ref(), Type::Unknown) {
                "unknown".to_string()
            } else {
                ts_type(k)
            };
            format!("Map<{}, {}>", key, ts_type(v))
        }
        Type::Option(inner) => format!("{} | null", ts_type_paren(inner)),
        Type::Tuple(elems) => {
            let parts: Vec<_> = elems.iter().map(ts_type).collect();
            format!("[{}]", parts.join(", "))
        }
        Type::Struct(name) | Type::Enum(name) => {
            let short = name.rsplit("::").next().unwrap_or(name);
            // AS3/JS `Object` is a dynamic property bag, not TypeScript's `Object`
            // interface. TypeScript's `Object` has no index signature, so any dynamic
            // key access causes TS7053. Map it to `Record<string, any>` instead.
            if short == "Object" {
                return "Record<string, any>".into();
            }
            // AS3 `Class` is the metaclass for all class objects. In TypeScript, class objects
            // are dynamically indexable (e.g. `MyClass["STATIC_FIELD"]`), so map to `any`.
            if short == "Class" {
                return "any".into();
            }
            // AS3 XML/XMLList have implicit string coercion — they are valid as index
            // keys and assignable to string fields.  TypeScript's XML class has no such
            // implicit coercion, so widen to `any` to avoid TS2538 and TS2322.
            if matches!(short, "XML" | "XMLList") {
                return "any".into();
            }
            sanitize_ident(short)
        }
        Type::ClassRef(_) => {
            // GML OBJT class names are used as integer object indices at runtime.
            // While TypeScript represents the class constructor as `typeof ClassName`,
            // callers of such a function get a misleading type. Since ClassRef values
            // are always widened to `as any` at their use sites, `any` is the correct
            // declared type for function signatures too.
            "any".into()
        }
        Type::Function(sig) => {
            let params: Vec<_> = sig
                .params
                .iter()
                .enumerate()
                .map(|(i, t)| format!("p{}: {}", i, ts_type(t)))
                .collect();
            format!("({}) => {}", params.join(", "), ts_type(&sig.return_ty))
        }
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => format!(
            "Generator<{}, {}, unknown>",
            ts_type(yield_ty),
            ts_type(return_ty)
        ),
        Type::Union(types) => {
            let mut parts = Vec::new();
            for t in types {
                let s = ts_type(t);
                if !parts.contains(&s) {
                    parts.push(s);
                }
            }
            parts.join(" | ")
        }
        Type::Var(_) => "unknown".into(),
        // TODO(phase-4): emit `unknown` once inference eliminates most Unknown
        // params.  Currently emits `any` to maintain behavioral equivalence;
        // switching to `unknown` surfaces ~21K TS18046/TS2345 errors because GML
        // params (_self, _other, loop vars) are typed Unknown rather than their
        // concrete class types.
        Type::Unknown => "any".into(),
    }
}

/// Map an IR [`Type`] to its TypeScript representation in a Flash-specific context.
///
/// Differs from [`ts_type`] in one way: `Map<Unknown, _>` → `"Dictionary"` (the
/// Flash runtime class that wraps `Map<unknown, unknown>` with a Proxy that supports
/// bracket-notation access).  Callers must ensure `Dictionary` is imported.
pub fn flash_ts_type(ty: &Type) -> String {
    match ty {
        // AS3 Dictionary is Map(Unknown, Unknown) in the IR but should be emitted
        // as `Dictionary` (the runtime class with index signatures) so that bracket
        // access `dict[key]` type-checks without TS7052.
        Type::Map(k, _) if matches!(k.as_ref(), Type::Unknown) => "Dictionary".into(),
        // AS3 Array allows both numeric and string indexing (it's a hash-array hybrid).
        // TypeScript's `any[]` only allows numeric indexing, causing TS7015 on string
        // keys. Emit `any` to allow all indexing patterns faithfully.
        Type::Array(_) => "any".into(),
        // AS3 XML/XMLList have implicit string coercion — they are valid as index
        // keys and assignable to string fields.  TypeScript's XML class has no such
        // implicit coercion, so declaring variables as `any` instead of `XML`/`XMLList`
        // avoids TS2538 (XML can't be used as index) and TS2322 (XML→string).
        Type::Struct(name)
            if matches!(name.rsplit("::").next().unwrap_or(name), "XML" | "XMLList") =>
        {
            "any".into()
        }
        _ => ts_type(ty),
    }
}

/// Map an IR [`Type`] to its TypeScript representation, using `class_names` to
/// resolve disambiguated class names when two classes share the same short name.
///
/// `class_names` maps qualified IR type names (e.g. `"classes.Items.Armors::GooArmor"`)
/// to the TypeScript identifier used in the emitted file (e.g. `"Armors_GooArmor"`).
/// Pass an empty map for contexts where no disambiguation is needed.
pub fn ts_type_with_names(ty: &Type, class_names: &HashMap<String, String>) -> String {
    match ty {
        Type::Struct(name) | Type::Enum(name) => {
            let short = name.rsplit("::").next().unwrap_or(name);
            if short == "Object" {
                return "Record<string, any>".into();
            }
            if short == "Class" {
                return "any".into();
            }
            class_names
                .get(name.as_str())
                .cloned()
                .unwrap_or_else(|| sanitize_ident(short))
        }
        _ => ts_type(ty),
    }
}

/// Like [`flash_ts_type`] but resolves disambiguated class names from `class_names`.
pub fn flash_ts_type_with_names(ty: &Type, class_names: &HashMap<String, String>) -> String {
    match ty {
        Type::Map(k, _) if matches!(k.as_ref(), Type::Unknown) => "Dictionary".into(),
        Type::Array(_) => "any".into(),
        Type::Struct(name)
            if matches!(name.rsplit("::").next().unwrap_or(name), "XML" | "XMLList") =>
        {
            "any".into()
        }
        _ => ts_type_with_names(ty, class_names),
    }
}

/// Wrap compound types in parens when used in contexts like `T[]`.
fn ts_type_paren(ty: &Type) -> String {
    match ty {
        Type::Option(_) | Type::Function(_) | Type::Union(_) => format!("({})", ts_type(ty)),
        _ => ts_type(ty),
    }
}
