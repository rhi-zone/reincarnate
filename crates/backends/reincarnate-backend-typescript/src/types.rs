use std::collections::HashMap;

use reincarnate_core::entity::PrimaryMap;
use reincarnate_core::ir::module::TypeDecl;
use reincarnate_core::ir::{Type, TypeId};

use crate::emit::sanitize_ident;
use crate::js_ast::{JsExpr, JsFunction, JsStmt};

/// Find a [`TypeId`] by name in `module_types` via a linear scan.
///
/// Used in emission contexts where only `module_types` (not the full `Module`) is
/// available but a name → TypeId lookup is needed (e.g. for runtime type names).
/// Returns `None` if no entry with the given name exists.
pub fn find_type_id(module_types: &PrimaryMap<TypeId, TypeDecl>, name: &str) -> Option<TypeId> {
    module_types
        .iter()
        .find(|(_, decl)| decl.name() == Some(name))
        .map(|(id, _)| id)
}

/// Resolve a [`TypeId`] to its short TypeScript identifier using `module_types`.
///
/// Returns the sanitized short name (e.g. `"Monster"` for `"classes::Monster"`),
/// or `"unknown"` when the id cannot be resolved.
pub fn ts_type_id(id: TypeId, module_types: &PrimaryMap<TypeId, TypeDecl>) -> String {
    if let Some(named) = module_types.get(id) {
        if let Some(name) = named.name() {
            let short = name.rsplit("::").next().unwrap_or(name);
            // AS3 `Object` is a dynamic property bag — same mapping as `Type::Struct("Object")`.
            if short == "Object" {
                return "Record<string, any>".into();
            }
            // AS3 `Class` metaclass — map to `any`.
            if short == "Class" {
                return "any".into();
            }
            // AS3 XML/XMLList — map to `any`.
            if matches!(short, "XML" | "XMLList") {
                return "any".into();
            }
            return sanitize_ident(short);
        }
    }
    "unknown".into()
}

/// Map an IR [`Type`] to its TypeScript representation.
///
/// For `Type::Instance(id)`, `module_types` is required to resolve the type name.
/// Use [`ts_type`] in contexts where `Instance` is guaranteed not to appear.
pub fn ts_type_with_module(ty: &Type, module_types: &PrimaryMap<TypeId, TypeDecl>) -> String {
    match ty {
        Type::Instance(id) => ts_type_id(*id, module_types),
        // Recurse into composite types so nested Instance(id) references are resolved.
        Type::Array(elem) => format!("{}[]", ts_type_paren_with_module(elem, module_types)),
        Type::Option(inner) => format!("{} | null", ts_type_paren_with_module(inner, module_types)),
        Type::Function(sig) => {
            let params: Vec<_> = sig
                .params
                .iter()
                .enumerate()
                .map(|(i, t)| format!("p{}: {}", i, ts_type_with_module(t, module_types)))
                .collect();
            format!(
                "({}) => {}",
                params.join(", "),
                ts_type_with_module(&sig.return_ty, module_types)
            )
        }
        Type::Union(types) => {
            let parts: Vec<_> = types
                .iter()
                .map(|t| ts_type_with_module(t, module_types))
                .collect();
            parts.join(" | ")
        }
        _ => ts_type(ty),
    }
}

fn ts_type_paren_with_module(ty: &Type, module_types: &PrimaryMap<TypeId, TypeDecl>) -> String {
    let s = ts_type_with_module(ty, module_types);
    if matches!(ty, Type::Union(_) | Type::Function(_)) {
        format!("({s})")
    } else {
        s
    }
}

/// Map an IR [`Type`] to its TypeScript representation.
///
/// For `Type::Instance(id)`, falls back to `"unknown"` — call [`ts_type_with_module`]
/// when struct/class field types (which are `Instance` after normalization) need
/// to be resolved to their real names.
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
        // Type::Instance is resolved via ts_type_with_module (which has module_types).
        // Bare ts_type() without module_types can't resolve the name, so fall back to unknown.
        Type::Instance(_) => "unknown".into(),
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
        Type::Unknown => "unknown".into(),
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
        // Instance(id) for XML/XMLList is handled by flash_ts_type_with_module.
        _ => ts_type(ty),
    }
}

/// Map an IR [`Type`] to its TypeScript representation for Flash in a context
/// where `module_types` is available to resolve [`Type::Instance`].
pub fn flash_ts_type_with_module(ty: &Type, module_types: &PrimaryMap<TypeId, TypeDecl>) -> String {
    match ty {
        Type::Map(k, _) if matches!(k.as_ref(), Type::Unknown) => "Dictionary".into(),
        Type::Array(_) => "any".into(),
        Type::Instance(id) => {
            if let Some(named) = module_types.get(*id) {
                if let Some(name) = named.name() {
                    let short = name.rsplit("::").next().unwrap_or(name);
                    if matches!(short, "XML" | "XMLList") {
                        return "any".into();
                    }
                }
            }
            ts_type_id(*id, module_types)
        }
        _ => flash_ts_type(ty),
    }
}

/// Map an IR [`Type`] to its TypeScript representation, using `class_names` to
/// resolve disambiguated class names when two classes share the same short name.
///
/// `class_names` maps qualified IR type names (e.g. `"classes.Items.Armors::GooArmor"`)
/// to the TypeScript identifier used in the emitted file (e.g. `"Armors_GooArmor"`).
/// Pass an empty map for contexts where no disambiguation is needed.
pub fn ts_type_with_names(ty: &Type, _class_names: &HashMap<String, String>) -> String {
    // Instance(id) is resolved by ts_type_with_names_and_module which has module_types.
    // All other types use ts_type directly.
    ts_type(ty)
}

/// Like [`ts_type_with_names`] but also resolves `Type::Instance(id)` via `module_types`.
///
/// Use this for struct/class field types and other types stored in the module
/// (which are `Instance` after `normalize_struct_types`).
pub fn ts_type_with_names_and_module(
    ty: &Type,
    class_names: &HashMap<String, String>,
    module_types: &PrimaryMap<TypeId, TypeDecl>,
) -> String {
    match ty {
        Type::Instance(id) => {
            if let Some(named) = module_types.get(*id) {
                if let Some(name) = named.name() {
                    let short = name.rsplit("::").next().unwrap_or(name);
                    if short == "Object" {
                        return "Record<string, any>".into();
                    }
                    if short == "Class" {
                        return "any".into();
                    }
                    if matches!(short, "XML" | "XMLList") {
                        return "any".into();
                    }
                    return class_names
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| sanitize_ident(short));
                }
            }
            "unknown".into()
        }
        _ => ts_type_with_names(ty, class_names),
    }
}

/// Like [`flash_ts_type_with_names`] but also resolves `Type::Instance(id)` via `module_types`.
pub fn flash_ts_type_with_names_and_module(
    ty: &Type,
    class_names: &HashMap<String, String>,
    module_types: &PrimaryMap<TypeId, TypeDecl>,
) -> String {
    match ty {
        Type::Map(k, _) if matches!(k.as_ref(), Type::Unknown) => "Dictionary".into(),
        Type::Array(_) => "any".into(),
        Type::Instance(id) => {
            if let Some(named) = module_types.get(*id) {
                if let Some(name) = named.name() {
                    let short = name.rsplit("::").next().unwrap_or(name);
                    if matches!(short, "XML" | "XMLList") {
                        return "any".into();
                    }
                    if short == "Object" {
                        return "Record<string, any>".into();
                    }
                    if short == "Class" {
                        return "any".into();
                    }
                    return class_names
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| sanitize_ident(short));
                }
            }
            "unknown".into()
        }
        _ => flash_ts_type_with_names(ty, class_names),
    }
}

/// Like [`flash_ts_type`] but resolves disambiguated class names from `class_names`.
pub fn flash_ts_type_with_names(ty: &Type, class_names: &HashMap<String, String>) -> String {
    match ty {
        Type::Map(k, _) if matches!(k.as_ref(), Type::Unknown) => "Dictionary".into(),
        Type::Array(_) => "any".into(),
        // Instance(id) for XML/XMLList is handled by flash_ts_type_with_names_and_module.
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

// ---------------------------------------------------------------------------
// Compound type normalization (recurse into Array/Map/Option/etc.)
// ---------------------------------------------------------------------------

/// Recursively normalize nested `Type::Instance` references inside compound types.
///
/// `Instance(id)` is preserved as-is — all downstream printing uses
/// `ts_type_with_module` which resolves the name via `module_types` at print
/// time.  This function recurses only to handle `Instance` inside `Array`,
/// `Option`, `Map`, etc.
#[allow(clippy::only_used_in_recursion)]
fn resolve_type(ty: Type, module_types: &PrimaryMap<TypeId, TypeDecl>) -> Type {
    match ty {
        // Instance(id) is the canonical post-build form — preserve it unchanged.
        Type::Instance(_) => ty,
        Type::Array(elem) => Type::Array(Box::new(resolve_type(*elem, module_types))),
        Type::Map(k, v) => Type::Map(
            Box::new(resolve_type(*k, module_types)),
            Box::new(resolve_type(*v, module_types)),
        ),
        Type::Option(inner) => Type::Option(Box::new(resolve_type(*inner, module_types))),
        Type::Tuple(elems) => Type::Tuple(
            elems
                .into_iter()
                .map(|t| resolve_type(t, module_types))
                .collect(),
        ),
        Type::Union(elems) => Type::Union(
            elems
                .into_iter()
                .map(|t| resolve_type(t, module_types))
                .collect(),
        ),
        Type::Function(sig) => {
            let params = sig
                .params
                .into_iter()
                .map(|t| resolve_type(t, module_types))
                .collect();
            let return_ty = resolve_type(sig.return_ty, module_types);
            Type::Function(Box::new(reincarnate_core::ir::ty::FunctionSig {
                params,
                return_ty,
                defaults: sig.defaults,
                has_rest_param: sig.has_rest_param,
                param_lower_bounds: vec![],
            }))
        }
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => Type::Coroutine {
            yield_ty: Box::new(resolve_type(*yield_ty, module_types)),
            return_ty: Box::new(resolve_type(*return_ty, module_types)),
        },
        // All other types are already in their final form.
        other => other,
    }
}

/// Resolve all `Type::Instance(id)` references in a `JsExpr` in-place.
fn resolve_expr_types(expr: &mut JsExpr, module_types: &PrimaryMap<TypeId, TypeDecl>) {
    match expr {
        JsExpr::Cast {
            expr: inner, ty, ..
        } => {
            resolve_expr_types(inner, module_types);
            *ty = resolve_type(ty.clone(), module_types);
        }
        JsExpr::TypeCheck {
            expr: inner, ty, ..
        } => {
            resolve_expr_types(inner, module_types);
            *ty = resolve_type(ty.clone(), module_types);
        }
        JsExpr::ArrowFunction {
            params,
            return_ty,
            body,
            ..
        } => {
            for (_, ty) in params.iter_mut() {
                *ty = resolve_type(ty.clone(), module_types);
            }
            *return_ty = resolve_type(return_ty.clone(), module_types);
            for stmt in body.iter_mut() {
                resolve_stmt_types(stmt, module_types);
            }
        }
        // Recurse into nested expressions.
        JsExpr::Binary { lhs, rhs, .. } | JsExpr::Cmp { lhs, rhs, .. } => {
            resolve_expr_types(lhs, module_types);
            resolve_expr_types(rhs, module_types);
        }
        JsExpr::Unary { expr: inner, .. }
        | JsExpr::Not(inner)
        | JsExpr::PostIncrement(inner)
        | JsExpr::Spread(inner)
        | JsExpr::TypeOf(inner)
        | JsExpr::GeneratorResume(inner) => {
            resolve_expr_types(inner, module_types);
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            resolve_expr_types(cond, module_types);
            resolve_expr_types(then_val, module_types);
            resolve_expr_types(else_val, module_types);
        }
        JsExpr::LogicalOr { lhs, rhs } | JsExpr::LogicalAnd { lhs, rhs } => {
            resolve_expr_types(lhs, module_types);
            resolve_expr_types(rhs, module_types);
        }
        JsExpr::Field { object, .. } => resolve_expr_types(object, module_types),
        JsExpr::Index { collection, index } => {
            resolve_expr_types(collection, module_types);
            resolve_expr_types(index, module_types);
        }
        JsExpr::Call { callee, args } => {
            resolve_expr_types(callee, module_types);
            for arg in args.iter_mut() {
                resolve_expr_types(arg, module_types);
            }
        }
        JsExpr::ArrayInit(elems) | JsExpr::TupleInit(elems) => {
            for elem in elems.iter_mut() {
                resolve_expr_types(elem, module_types);
            }
        }
        JsExpr::ObjectInit(kvs) => {
            for (_, val) in kvs.iter_mut() {
                resolve_expr_types(val, module_types);
            }
        }
        JsExpr::NullCoalesceAssign { target, value } => {
            resolve_expr_types(target, module_types);
            resolve_expr_types(value, module_types);
        }
        JsExpr::Assign { lhs, rhs } => {
            resolve_expr_types(lhs, module_types);
            resolve_expr_types(rhs, module_types);
        }
        JsExpr::SystemCall { args, .. } => {
            for arg in args.iter_mut() {
                resolve_expr_types(arg, module_types);
            }
        }
        JsExpr::GeneratorCreate { args, .. } => {
            for arg in args.iter_mut() {
                resolve_expr_types(arg, module_types);
            }
        }
        JsExpr::Yield(Some(inner)) => resolve_expr_types(inner, module_types),
        JsExpr::Yield(None) => {}
        JsExpr::New { callee, args } => {
            resolve_expr_types(callee, module_types);
            for arg in args.iter_mut() {
                resolve_expr_types(arg, module_types);
            }
        }
        JsExpr::In { key, object } => {
            resolve_expr_types(key, module_types);
            resolve_expr_types(object, module_types);
        }
        JsExpr::Delete { object, key } => {
            resolve_expr_types(object, module_types);
            resolve_expr_types(key, module_types);
        }
        JsExpr::SuperCall(args) | JsExpr::SuperMethodCall { args, .. } => {
            for arg in args.iter_mut() {
                resolve_expr_types(arg, module_types);
            }
        }
        JsExpr::SuperSet { value, .. } => resolve_expr_types(value, module_types),
        JsExpr::LooseEq { lhs, rhs } | JsExpr::LooseNe { lhs, rhs } => {
            resolve_expr_types(lhs, module_types);
            resolve_expr_types(rhs, module_types);
        }
        JsExpr::NonNull(inner) => resolve_expr_types(inner, module_types),
        // Leaf expressions — no types to resolve.
        JsExpr::Literal(_)
        | JsExpr::Var(_)
        | JsExpr::This
        | JsExpr::Activation
        | JsExpr::SuperGet(_) => {}
    }
}

/// Resolve all `Type::Instance(id)` references in a `JsStmt` in-place.
fn resolve_stmt_types(stmt: &mut JsStmt, module_types: &PrimaryMap<TypeId, TypeDecl>) {
    match stmt {
        JsStmt::VarDecl { ty, init, .. } => {
            if let Some(t) = ty {
                *t = resolve_type(t.clone(), module_types);
            }
            if let Some(e) = init {
                resolve_expr_types(e, module_types);
            }
        }
        JsStmt::Assign { target, value } | JsStmt::CompoundAssign { target, value, .. } => {
            resolve_expr_types(target, module_types);
            resolve_expr_types(value, module_types);
        }
        JsStmt::Expr(e) => resolve_expr_types(e, module_types),
        JsStmt::Return(Some(e)) => resolve_expr_types(e, module_types),
        JsStmt::Return(None) => {}
        JsStmt::Throw(e) => resolve_expr_types(e, module_types),
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            resolve_expr_types(cond, module_types);
            for s in then_body.iter_mut() {
                resolve_stmt_types(s, module_types);
            }
            for s in else_body.iter_mut() {
                resolve_stmt_types(s, module_types);
            }
        }
        JsStmt::While { cond, body } => {
            resolve_expr_types(cond, module_types);
            for s in body.iter_mut() {
                resolve_stmt_types(s, module_types);
            }
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            for s in init.iter_mut() {
                resolve_stmt_types(s, module_types);
            }
            resolve_expr_types(cond, module_types);
            for s in update.iter_mut() {
                resolve_stmt_types(s, module_types);
            }
            for s in body.iter_mut() {
                resolve_stmt_types(s, module_types);
            }
        }
        JsStmt::Loop { body } => {
            for s in body.iter_mut() {
                resolve_stmt_types(s, module_types);
            }
        }
        JsStmt::ForOf {
            binding_ty,
            iterable,
            body,
            ..
        } => {
            if let Some(t) = binding_ty {
                *t = resolve_type(t.clone(), module_types);
            }
            resolve_expr_types(iterable, module_types);
            for s in body.iter_mut() {
                resolve_stmt_types(s, module_types);
            }
        }
        JsStmt::Dispatch { blocks, .. } => {
            for (_, block_stmts) in blocks.iter_mut() {
                for s in block_stmts.iter_mut() {
                    resolve_stmt_types(s, module_types);
                }
            }
        }
        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => {
            resolve_expr_types(value, module_types);
            for (case_expr, case_body) in cases.iter_mut() {
                resolve_expr_types(case_expr, module_types);
                for s in case_body.iter_mut() {
                    resolve_stmt_types(s, module_types);
                }
            }
            for s in default_body.iter_mut() {
                resolve_stmt_types(s, module_types);
            }
        }
        JsStmt::Break | JsStmt::Continue | JsStmt::LabeledBreak { .. } => {}
    }
}

/// Resolve all `Type::Instance(id)` references in a `JsFunction` in-place.
///
/// This converts `TypeId`-based Instance types to their string names so that
/// downstream type-processing functions (ts_type, rename_type_with_map, etc.)
/// only see the string-keyed `Struct(name)` form they already handle.
///
/// Call this immediately after lowering from IR to JsAST and before any
/// engine-specific rewrites or printing.
pub fn resolve_js_function_types(
    func: &mut JsFunction,
    module_types: &PrimaryMap<TypeId, TypeDecl>,
) {
    for (_, ty) in func.params.iter_mut() {
        *ty = resolve_type(ty.clone(), module_types);
    }
    func.return_ty = resolve_type(func.return_ty.clone(), module_types);
    for stmt in func.body.iter_mut() {
        resolve_stmt_types(stmt, module_types);
    }
}
