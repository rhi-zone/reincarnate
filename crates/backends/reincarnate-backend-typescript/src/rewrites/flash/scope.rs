//! Scope lookup detection & resolution.

use reincarnate_core::ir::{CastKind, Constant, Type};

use crate::js_ast::JsExpr;

use super::context::{FlashRewriteCtx, ScopeResolution};

/// Check whether a JsExpr is a Flash scope-lookup SystemCall.
pub(super) fn is_scope_lookup(expr: &JsExpr) -> bool {
    scope_lookup_args(expr).is_some()
}

/// Extract the args from a scope-lookup SystemCall, or None.
pub(super) fn scope_lookup_args(expr: &JsExpr) -> Option<&[JsExpr]> {
    match expr {
        JsExpr::SystemCall {
            system,
            method,
            args,
        } if system == "Flash.Scope"
            && (method == "findPropStrict" || method == "findProperty") =>
        {
            Some(args)
        }
        _ => None,
    }
}

/// Extract the class name from a scope-lookup arg string constant.
///
/// For arg like `"classes:SomeClass::someField"`, returns `"SomeClass"`.
pub(super) fn class_from_scope_arg(args: &[JsExpr]) -> Option<String> {
    let arg = args.first()?;
    if let JsExpr::Literal(Constant::String(s)) = arg {
        let prefix = s.rsplit_once("::")?.0;
        let class_name = prefix.rsplit_once(':')?.1;
        Some(class_name.to_string())
    } else {
        None
    }
}

pub(super) fn resolve_scope_lookup(args: &[JsExpr], ctx: &FlashRewriteCtx) -> ScopeResolution {
    if let Some(class_name) = class_from_scope_arg(args) {
        if ctx.ancestors.contains(&class_name) {
            return ScopeResolution::Ancestor(class_name);
        }
    }
    ScopeResolution::ScopeLookup
}

/// Resolve `Field { object: scope_lookup, field }`.
pub(super) fn resolve_field(object: &JsExpr, field: &str, ctx: &FlashRewriteCtx) -> Option<JsExpr> {
    let args = scope_lookup_args(object)?;
    let effective = field.rsplit("::").next().unwrap_or(field);

    Some(match resolve_scope_lookup(args, ctx) {
        ScopeResolution::Ancestor(ref class_name) => {
            if ctx.is_cinit
                || ctx.instance_fields.contains(effective)
                || ctx.method_names.contains(effective)
            {
                JsExpr::Field {
                    object: Box::new(JsExpr::This),
                    field: effective.to_string(),
                }
            } else {
                JsExpr::Field {
                    object: Box::new(JsExpr::Var(class_name.clone())),
                    field: effective.to_string(),
                }
            }
        }
        ScopeResolution::ScopeLookup => {
            // cinit static-field assignments take priority over class-name lookups:
            // `Edryn` may be both a known class AND a static field on the current
            // class — in cinit context the field wins (→ `this.Edryn`).
            if ctx.is_cinit && ctx.static_fields.contains(effective) {
                JsExpr::Field {
                    object: Box::new(JsExpr::This),
                    field: effective.to_string(),
                }
            } else if let Some(short) = ctx.class_names.get(field).or_else(|| {
                // `lower_field` strips the namespace from the field name before lowering to
                // JsExpr, so `field` may be the bare short name (e.g. `"GooArmor"`) rather
                // than the fully-qualified name.  The scope-lookup arg retains the full
                // qualified name, so try that as a fallback key.
                args.first().and_then(|a| {
                    if let JsExpr::Literal(Constant::String(s)) = a {
                        ctx.class_names.get(s.as_str())
                    } else {
                        None
                    }
                })
            }) {
                // Full-qualified class name → short import alias.
                return Some(JsExpr::Var(short.clone()));
            } else if ctx.known_classes.contains(effective) {
                // Short class name (e.g. `PerkClass`) → bare Var, not activation-prefixed.
                return Some(JsExpr::Var(effective.to_string()));
            } else if let Some(ref class_name) = ctx.class_short_name {
                if ctx.const_instance_fields.contains(effective) {
                    JsExpr::Field {
                        object: Box::new(JsExpr::Var(class_name.clone())),
                        field: effective.to_string(),
                    }
                } else if let Some(owner) = ctx.static_field_owners.get(effective) {
                    JsExpr::Field {
                        object: Box::new(JsExpr::Var(owner.clone())),
                        field: effective.to_string(),
                    }
                } else if ctx.has_self
                    && (ctx.instance_fields.contains(effective)
                        || ctx.method_names.contains(effective))
                {
                    JsExpr::Field {
                        object: Box::new(JsExpr::This),
                        field: effective.to_string(),
                    }
                } else if let Some(ref av) = ctx.activation_var {
                    if ctx.activation_slots.contains(effective) {
                        JsExpr::Field {
                            object: Box::new(JsExpr::Var(av.clone())),
                            field: effective.to_string(),
                        }
                    } else {
                        JsExpr::Var(effective.to_string())
                    }
                } else {
                    JsExpr::Var(effective.to_string())
                }
            } else if ctx.has_self
                && (ctx.instance_fields.contains(effective) || ctx.method_names.contains(effective))
            {
                JsExpr::Field {
                    object: Box::new(JsExpr::This),
                    field: effective.to_string(),
                }
            } else if let Some(ref av) = ctx.activation_var {
                if ctx.activation_slots.contains(effective) {
                    JsExpr::Field {
                        object: Box::new(JsExpr::Var(av.clone())),
                        field: effective.to_string(),
                    }
                } else {
                    JsExpr::Var(effective.to_string())
                }
            } else {
                JsExpr::Var(effective.to_string())
            }
        }
    })
}

/// Resolve a Call where the callee is `Field(scope_lookup, method)`.
pub(super) fn resolve_scope_call(
    method: &str,
    scope_args: &[JsExpr],
    rest_args: Vec<JsExpr>,
    ctx: &FlashRewriteCtx,
) -> JsExpr {
    let effective = method.rsplit("::").next().unwrap_or(method);

    let callee = match resolve_scope_lookup(scope_args, ctx) {
        ScopeResolution::Ancestor(ref class_name) => {
            let is_instance =
                ctx.instance_fields.contains(effective) || ctx.method_names.contains(effective);
            if ctx.is_cinit || is_instance {
                // Instance method/field-as-callable (or cinit scope) → this.method
                JsExpr::Field {
                    object: Box::new(JsExpr::This),
                    field: effective.to_string(),
                }
            } else {
                // Static method on ancestor class → ClassName.method
                JsExpr::Field {
                    object: Box::new(JsExpr::Var(class_name.clone())),
                    field: effective.to_string(),
                }
            }
        }
        ScopeResolution::ScopeLookup => {
            // Non-ancestor: try to extract a class name for static dispatch.
            if let Some(class_name) = class_from_scope_arg(scope_args) {
                JsExpr::Field {
                    object: Box::new(JsExpr::Var(class_name)),
                    field: effective.to_string(),
                }
            } else if ctx.has_self
                && (ctx.method_names.contains(effective) || ctx.instance_fields.contains(effective))
                || ctx.is_cinit
            {
                JsExpr::Field {
                    object: Box::new(JsExpr::This),
                    field: effective.to_string(),
                }
            } else if let Some(owner) = ctx.static_method_owners.get(effective) {
                // Static method found in ancestor hierarchy → OwnerClass.method
                JsExpr::Field {
                    object: Box::new(JsExpr::Var(owner.clone())),
                    field: effective.to_string(),
                }
            } else if let Some(ref av) = ctx.activation_var {
                if ctx.activation_slots.contains(effective) {
                    JsExpr::Field {
                        object: Box::new(JsExpr::Var(av.clone())),
                        field: effective.to_string(),
                    }
                } else {
                    JsExpr::Var(effective.to_string())
                }
            } else {
                JsExpr::Var(effective.to_string())
            }
        }
    };

    // Class coercion: ClassName(arg) → asType(arg, ClassName)
    // AS3 allows `ClassName(obj)` as a type coercion (returns obj if instance, null otherwise).
    // In JS, calling a class constructor without `new` throws — emit asType instead.
    if rest_args.len() == 1 {
        if let JsExpr::Var(ref name) = callee {
            if ctx.known_classes.contains(name.as_str()) {
                return JsExpr::Cast {
                    expr: Box::new(rest_args.into_iter().next().unwrap()),
                    ty: Type::Struct(name.clone()),
                    kind: CastKind::NullableCoerce,
                };
            }
        }
    }

    JsExpr::Call {
        callee: Box::new(callee),
        args: rest_args,
    }
}
