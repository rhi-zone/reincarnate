use std::collections::HashMap;

use reincarnate_core::entity::{EntityRef, PrimaryMap};
use reincarnate_core::ir::module::NamedType;
use reincarnate_core::ir::{Type, TypeId};

use crate::emit::sanitize_ident;
use crate::js_ast::{JsExpr, JsFunction, JsStmt};

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
        // Instance(id) appears only in raw IR; after resolve_js_function_types() all
        // Instance types are converted to Struct. If Instance reaches ts_type() it
        // means the resolution step was skipped — fall back to "unknown" to avoid a
        // panic while still flagging the gap.
        Type::Instance(_) => "unknown".into(),
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

// ---------------------------------------------------------------------------
// Instance type resolution — converts Type::Instance(TypeId) → Type::Struct(name)
// ---------------------------------------------------------------------------

/// Recursively convert `Type::Instance(id)` → `Type::Struct(name)` in a `Type`.
///
/// Called before emitting TypeScript so that all downstream type-processing
/// functions (ts_type, rename_type_with_map, collect_type_ref, etc.) only see
/// the string-keyed `Struct(name)` form they already handle.
fn resolve_type(ty: Type, module_types: &PrimaryMap<TypeId, NamedType>) -> Type {
    match ty {
        Type::Instance(id) => {
            let name = module_types
                .get(id)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| format!("type{}", id.index()));
            Type::Struct(name)
        }
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
fn resolve_expr_types(expr: &mut JsExpr, module_types: &PrimaryMap<TypeId, NamedType>) {
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
fn resolve_stmt_types(stmt: &mut JsStmt, module_types: &PrimaryMap<TypeId, NamedType>) {
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
    module_types: &PrimaryMap<TypeId, NamedType>,
) {
    for (_, ty) in func.params.iter_mut() {
        *ty = resolve_type(ty.clone(), module_types);
    }
    func.return_ty = resolve_type(func.return_ty.clone(), module_types);
    for stmt in func.body.iter_mut() {
        resolve_stmt_types(stmt, module_types);
    }
}
