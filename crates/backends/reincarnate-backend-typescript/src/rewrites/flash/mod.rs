//! Flash/AVM2-specific JsExpr → JsExpr rewrite pass.
//!
//! Runs AFTER mechanical `Expr → JsExpr` lowering. Resolves Flash SystemCall
//! nodes (scope lookups, super dispatch, object construction, etc.) into
//! native JavaScript constructs. All Flash-specific knowledge is confined here;
//! `lower.rs` is purely engine-agnostic.

mod context;
mod dead_activation;
mod expr;
mod method_bind;
mod scope;
mod stmt;
mod super_hoist;

use std::collections::{BTreeMap, HashMap, HashSet};

use reincarnate_core::ir::{ExternalImport, ValueId};

use crate::emit::{ClassRegistry, RefSets};
use crate::js_ast::{JsExpr, JsStmt};

pub use context::FlashRewriteCtx;
pub use dead_activation::eliminate_dead_activations;
pub use super_hoist::hoist_super_call;

use method_bind::bind_method_refs_stmts;
use stmt::rewrite_stmts;

use crate::js_ast::JsFunction;

// ---------------------------------------------------------------------------
// Activation-scope detection (for functions with closures)
// ---------------------------------------------------------------------------

/// Scan a statement list (top-level only) for the first `VarDecl` whose init
/// is a `Flash.Scope.newActivation` SystemCall.  Returns the variable name.
fn find_activation_var(stmts: &[JsStmt]) -> Option<String> {
    for stmt in stmts {
        if let JsStmt::VarDecl {
            name,
            init:
                Some(JsExpr::SystemCall {
                    system,
                    method,
                    args,
                }),
            ..
        } = stmt
        {
            if system == "Flash.Scope" && method == "newActivation" && args.is_empty() {
                return Some(name.clone());
            }
        }
    }
    None
}

/// Collect all field names assigned to the activation-scope object.
fn collect_activation_slots(stmts: &[JsStmt], activation_var: &str) -> HashSet<String> {
    let mut slots = HashSet::new();
    for stmt in stmts {
        if let JsStmt::Assign {
            target: JsExpr::Field { object, field },
            ..
        } = stmt
        {
            if matches!(object.as_ref(), JsExpr::Var(v) if v == activation_var) {
                slots.insert(field.clone());
            }
        }
    }
    slots
}

// ---------------------------------------------------------------------------
// Top-level rewrite entry point
// ---------------------------------------------------------------------------

/// Rewrite a lowered JS function, resolving all Flash SystemCalls and
/// scope-lookup patterns.
pub fn rewrite_flash_function(mut func: JsFunction, ctx: &FlashRewriteCtx) -> JsFunction {
    // Auto-detect activation scope for functions with closures (those that
    // still emit the SystemCall("Flash.Scope", "newActivation") pattern).
    if ctx.activation_var.is_none() {
        if let Some(av) = find_activation_var(&func.body) {
            let slots = collect_activation_slots(&func.body, &av);
            let ctx2 = FlashRewriteCtx {
                activation_var: Some(av),
                activation_slots: slots,
                class_names: ctx.class_names.clone(),
                ancestors: ctx.ancestors.clone(),
                method_names: ctx.method_names.clone(),
                instance_fields: ctx.instance_fields.clone(),
                has_self: ctx.has_self,
                suppress_super: ctx.suppress_super,
                parent_is_runtime: ctx.parent_is_runtime,
                is_cinit: ctx.is_cinit,
                is_constructor: ctx.is_constructor,
                is_static: ctx.is_static,
                static_fields: ctx.static_fields.clone(),
                static_method_owners: ctx.static_method_owners.clone(),
                static_field_owners: ctx.static_field_owners.clone(),
                const_instance_fields: ctx.const_instance_fields.clone(),
                class_short_name: ctx.class_short_name.clone(),
                bindable_methods: ctx.bindable_methods.clone(),
                closure_bodies: ctx.closure_bodies.clone(),
                known_classes: ctx.known_classes.clone(),
                unique_static_fields: ctx.unique_static_fields.clone(),
            };
            func.body = rewrite_stmts(func.body, &ctx2);
            if ctx2.has_self && !ctx2.bindable_methods.is_empty() {
                bind_method_refs_stmts(&mut func.body, &ctx2.bindable_methods);
            }
            return func;
        }
    }
    func.body = rewrite_stmts(func.body, ctx);
    if ctx.has_self && !ctx.bindable_methods.is_empty() {
        bind_method_refs_stmts(&mut func.body, &ctx.bindable_methods);
    }
    func
}

// ---------------------------------------------------------------------------
// Import extraction for Flash scope-lookup SystemCalls
// ---------------------------------------------------------------------------

/// Collect import references from Flash.Scope findPropStrict/findProperty calls.
///
/// Scope lookups may resolve to static methods/fields on ancestor classes, class
/// coercions, or module-level globals. This produces the value/type imports that
/// the emitter needs.
#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_flash_scope_refs(
    args: &[ValueId],
    const_strings: &HashMap<ValueId, &str>,
    self_name: &str,
    registry: &ClassRegistry,
    external_imports: &BTreeMap<String, ExternalImport>,
    static_method_owners: &HashMap<String, String>,
    static_field_owners: &HashMap<String, String>,
    global_names: &HashSet<String>,
    refs: &mut RefSets,
) {
    if let Some(&scope_str) = args.first().and_then(|v| const_strings.get(v)) {
        // Extract the bare name from the scope arg.
        let bare = scope_str.rsplit("::").next().unwrap_or(scope_str);
        if let Some(owner) = static_method_owners.get(bare) {
            if owner != self_name {
                if let Some(entry) = registry.lookup(owner) {
                    refs.value_refs.insert(entry.short_name.clone());
                }
            }
        }
        if let Some(owner) = static_field_owners.get(bare) {
            if owner != self_name {
                if let Some(entry) = registry.lookup(owner) {
                    refs.value_refs.insert(entry.short_name.clone());
                }
            }
        }
        // Class coercion: FindPropStrict("ClassName") + CallPropLex("ClassName", 1)
        // resolves to asType(obj, ClassName) — need the class as a value import.
        if bare != self_name {
            if let Some(entry) = registry.lookup(bare) {
                refs.value_refs.insert(entry.short_name.clone());
            } else if external_imports.contains_key(scope_str) {
                // External runtime class (e.g. DisplayObject from flash/display).
                refs.ext_value_refs.insert(scope_str.to_string());
            }
        }
        // Module-level globals (package variables).
        if global_names.contains(bare) {
            refs.globals_used.insert(bare.to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Flash namespace helpers (used by emit.rs warn_unmapped_reference)
// ---------------------------------------------------------------------------

/// Returns `true` for Flash package namespaces that are known non-runtime
/// authoring-library packages (`fl.*`) or similar known prefixes that should
/// not produce "unmapped external reference" warnings.  Flash runtime stdlib
/// packages (`flash.*`) are NOT included here — callers handle those separately
/// to emit a more actionable "not in runtime stdlib" message.
pub fn is_known_flash_namespace(ns: &str) -> bool {
    ns.starts_with("fl.")
}

#[cfg(test)]
mod tests {
    use reincarnate_core::ir::{CastKind, Constant, Type};

    use crate::js_ast::{JsExpr, JsStmt};

    use super::context::FlashRewriteCtx;
    use super::expr::{rewrite_expr, rewrite_system_call};
    use super::method_bind::bind_method_refs_expr;
    use super::scope::{resolve_field, resolve_scope_call};
    use super::stmt::rewrite_stmt;
    use super::super_hoist::hoist_super_call;

    use std::collections::{HashMap, HashSet};

    /// Build a minimal `FlashRewriteCtx` with all fields empty.
    fn empty_ctx() -> FlashRewriteCtx {
        FlashRewriteCtx {
            class_names: HashMap::new(),
            ancestors: HashSet::new(),
            method_names: HashSet::new(),
            instance_fields: HashSet::new(),
            has_self: false,
            suppress_super: false,
            parent_is_runtime: false,
            is_cinit: false,
            is_constructor: false,
            is_static: false,
            static_fields: HashSet::new(),
            static_method_owners: HashMap::new(),
            static_field_owners: HashMap::new(),
            const_instance_fields: HashSet::new(),
            class_short_name: None,
            bindable_methods: HashSet::new(),
            closure_bodies: HashMap::new(),
            known_classes: HashSet::new(),
            unique_static_fields: HashMap::new(),
            activation_var: None,
            activation_slots: HashSet::new(),
        }
    }

    fn scope_lookup(name: &str) -> JsExpr {
        JsExpr::SystemCall {
            system: "Flash.Scope".into(),
            method: "findPropStrict".into(),
            args: vec![JsExpr::Literal(Constant::String(name.into()))],
        }
    }

    fn body_stmt(name: &str) -> JsStmt {
        JsStmt::Expr(JsExpr::Call {
            callee: Box::new(JsExpr::Var(name.into())),
            args: vec![],
        })
    }

    // --- Scope resolution ---

    #[test]
    fn scope_lookup_ancestor_resolves_to_this_for_instance_field() {
        let mut ctx = empty_ctx();
        ctx.ancestors.insert("MyClass".into());
        ctx.instance_fields.insert("x".into());
        let result = resolve_field(&scope_lookup("classes:MyClass::x"), "x", &ctx);
        assert!(result.is_some());
        let expr = result.unwrap();
        assert!(matches!(&expr, JsExpr::Field { object, field }
            if matches!(object.as_ref(), JsExpr::This) && field == "x"));
    }

    #[test]
    fn scope_lookup_ancestor_static_resolves_to_class_dot_field() {
        let mut ctx = empty_ctx();
        ctx.ancestors.insert("MyClass".into());
        let result = resolve_field(&scope_lookup("classes:MyClass::MAX"), "MAX", &ctx);
        assert!(result.is_some());
        let expr = result.unwrap();
        assert!(matches!(&expr, JsExpr::Field { object, field }
            if matches!(object.as_ref(), JsExpr::Var(n) if n == "MyClass") && field == "MAX"));
    }

    #[test]
    fn scope_lookup_non_ancestor_resolves_to_bare_var() {
        let ctx = empty_ctx();
        let result = resolve_field(&scope_lookup("global::trace"), "trace", &ctx);
        assert!(result.is_some());
        let expr = result.unwrap();
        assert!(matches!(&expr, JsExpr::Var(n) if n == "trace"));
    }

    #[test]
    fn scope_lookup_with_self_and_method_resolves_to_this() {
        let mut ctx = empty_ctx();
        ctx.has_self = true;
        ctx.method_names.insert("update".into());
        let result = resolve_field(&scope_lookup("global::update"), "update", &ctx);
        assert!(result.is_some());
        let expr = result.unwrap();
        assert!(matches!(&expr, JsExpr::Field { object, field }
            if matches!(object.as_ref(), JsExpr::This) && field == "update"));
    }

    // --- SystemCall rewrites ---

    #[test]
    fn construct_super_becomes_super_call() {
        let ctx = empty_ctx();
        let args = vec![JsExpr::This, JsExpr::Literal(Constant::Int(42))];
        let result = rewrite_system_call("Flash.Class", "constructSuper", &args, &ctx);
        assert!(result.is_some());
        let expr = result.unwrap();
        // First arg is _shims (injected), second is the original arg (42).
        assert!(matches!(&expr, JsExpr::SuperCall(a) if a.len() == 2));
        if let JsExpr::SuperCall(a) = &expr {
            assert!(matches!(&a[0], JsExpr::Var(n) if n == "_shims"));
        }
    }

    #[test]
    fn construct_super_suppressed_becomes_null() {
        let mut ctx = empty_ctx();
        ctx.suppress_super = true;
        let args = vec![JsExpr::This];
        let result = rewrite_system_call("Flash.Class", "constructSuper", &args, &ctx);
        assert!(matches!(result, Some(JsExpr::Literal(Constant::Null))));
    }

    #[test]
    fn construct_becomes_new() {
        let mut ctx = empty_ctx();
        // MyClass is a user-defined class → _shims should be injected.
        ctx.class_names
            .insert("classes::MyClass".into(), "MyClass".into());
        let args = vec![
            JsExpr::Var("MyClass".into()),
            JsExpr::Literal(Constant::Int(1)),
        ];
        let result = rewrite_system_call("Flash.Object", "construct", &args, &ctx);
        assert!(result.is_some());
        let expr = result.unwrap();
        // First arg is this._shims (injected), second is the original arg (1).
        assert!(matches!(&expr, JsExpr::New { callee, args }
            if matches!(callee.as_ref(), JsExpr::Var(n) if n == "MyClass")
            && args.len() == 2));
        if let JsExpr::New { args, .. } = &expr {
            assert!(matches!(&args[0], JsExpr::Field { object, field }
                if matches!(object.as_ref(), JsExpr::This) && field == "_shims"));
        }
    }

    #[test]
    fn construct_object_no_args_becomes_empty_object() {
        let ctx = empty_ctx();
        let args = vec![JsExpr::Var("Object".into())];
        let result = rewrite_system_call("Flash.Object", "construct", &args, &ctx);
        assert!(matches!(result, Some(JsExpr::ObjectInit(pairs)) if pairs.is_empty()));
    }

    #[test]
    fn generic_shim_rewrites_to_this_shims_in_instance_context() {
        let mut ctx = empty_ctx();
        ctx.has_self = true;
        let args = vec![
            JsExpr::Literal(Constant::Int(0)),
            JsExpr::Literal(Constant::Int(0)),
        ];
        let result = rewrite_system_call("renderer", "clear", &args, &ctx);
        assert!(result.is_some(), "should rewrite generic shim system call");
        // Result should be a Call expression whose callee accesses this._shims.renderer.clear
        if let Some(JsExpr::Call {
            callee,
            args: call_args,
        }) = result
        {
            assert_eq!(call_args.len(), 2);
            // callee = this._shims.renderer.clear
            assert!(
                matches!(callee.as_ref(),
                    JsExpr::Field { object, field }
                    if field == "clear" && matches!(object.as_ref(),
                        JsExpr::Field { object: o2, field: f2 }
                        if f2 == "renderer" && matches!(o2.as_ref(),
                            JsExpr::Field { object: o3, field: f3 }
                            if f3 == "_shims" && matches!(o3.as_ref(), JsExpr::This)
                        )
                    )
                ),
                "callee should be this._shims.renderer.clear"
            );
        } else {
            panic!("expected Call expression");
        }
    }

    #[test]
    fn generic_shim_not_rewritten_without_self() {
        let ctx = empty_ctx(); // has_self defaults to false in empty_ctx
        let args = vec![];
        let result = rewrite_system_call("renderer", "clear", &args, &ctx);
        // Should fall through (None) when not in instance context
        assert!(
            result.is_none(),
            "should not rewrite shim call without self"
        );
    }

    #[test]
    fn typeof_rewrite() {
        let ctx = empty_ctx();
        let args = vec![JsExpr::Var("x".into())];
        let result = rewrite_system_call("Flash.Object", "typeOf", &args, &ctx);
        assert!(matches!(result, Some(JsExpr::TypeOf(_))));
    }

    #[test]
    fn has_property_becomes_in() {
        let ctx = empty_ctx();
        let args = vec![
            JsExpr::Var("obj".into()),
            JsExpr::Literal(Constant::String("key".into())),
        ];
        let result = rewrite_system_call("Flash.Object", "hasProperty", &args, &ctx);
        assert!(matches!(result, Some(JsExpr::In { .. })));
    }

    #[test]
    fn delete_property_becomes_delete() {
        let ctx = empty_ctx();
        let args = vec![
            JsExpr::Var("obj".into()),
            JsExpr::Literal(Constant::String("key".into())),
        ];
        let result = rewrite_system_call("Flash.Object", "deleteProperty", &args, &ctx);
        assert!(matches!(result, Some(JsExpr::Delete { .. })));
    }

    #[test]
    fn new_object_becomes_object_init() {
        let ctx = empty_ctx();
        let args = vec![
            JsExpr::Literal(Constant::String("a".into())),
            JsExpr::Literal(Constant::Int(1)),
            JsExpr::Literal(Constant::String("b".into())),
            JsExpr::Literal(Constant::Int(2)),
        ];
        let result = rewrite_system_call("Flash.Object", "newObject", &args, &ctx);
        assert!(result.is_some());
        let expr = result.unwrap();
        if let JsExpr::ObjectInit(pairs) = expr {
            assert_eq!(pairs.len(), 2);
            assert_eq!(pairs[0].0, "a");
            assert_eq!(pairs[1].0, "b");
        } else {
            panic!("expected ObjectInit, got {:?}", expr);
        }
    }

    #[test]
    fn call_super_becomes_super_method_call() {
        let ctx = empty_ctx();
        let args = vec![
            JsExpr::This,
            JsExpr::Literal(Constant::String("ns::doStuff".into())),
            JsExpr::Literal(Constant::Int(42)),
        ];
        let result = rewrite_system_call("Flash.Class", "callSuper", &args, &ctx);
        assert!(result.is_some());
        if let Some(JsExpr::SuperMethodCall { method, args }) = result {
            assert_eq!(method, "doStuff");
            assert_eq!(args.len(), 1);
        } else {
            panic!("expected SuperMethodCall");
        }
    }

    #[test]
    fn get_super_becomes_super_get() {
        let ctx = empty_ctx();
        let args = vec![
            JsExpr::This,
            JsExpr::Literal(Constant::String("value".into())),
        ];
        let result = rewrite_system_call("Flash.Class", "getSuper", &args, &ctx);
        assert!(matches!(result, Some(JsExpr::SuperGet(n)) if n == "value"));
    }

    #[test]
    fn apply_type_becomes_array() {
        let ctx = empty_ctx();
        let result = rewrite_system_call("Flash.Object", "applyType", &[], &ctx);
        assert!(matches!(result, Some(JsExpr::Var(n)) if n == "Array"));
    }

    #[test]
    fn class_coercion_in_scope_call() {
        let mut ctx = empty_ctx();
        ctx.known_classes.insert("Sprite".into());
        // When scope arg has no class prefix, callee resolves to bare Var("Sprite")
        // which triggers the class coercion path.
        let rewritten = resolve_scope_call(
            "Sprite",
            &[JsExpr::Literal(Constant::String("Sprite".into()))],
            vec![JsExpr::Var("obj".into())],
            &ctx,
        );
        assert!(matches!(
            &rewritten,
            JsExpr::Cast {
                kind: CastKind::NullableCoerce,
                ..
            }
        ));
    }

    // --- hoist_super_call ---

    #[test]
    fn hoist_super_call_no_deps() {
        let mut body = vec![
            body_stmt("a"),
            body_stmt("b"),
            JsStmt::Expr(JsExpr::SuperCall(vec![])),
        ];
        hoist_super_call(&mut body, None);
        assert!(matches!(&body[0], JsStmt::Expr(JsExpr::SuperCall(_))));
    }

    #[test]
    fn hoist_super_call_with_dep() {
        let mut body = vec![
            JsStmt::VarDecl {
                name: "x".into(),
                ty: Some(Type::Int(32)),
                init: Some(JsExpr::Literal(Constant::Int(1))),
                mutable: false,
            },
            body_stmt("other"),
            JsStmt::Expr(JsExpr::SuperCall(vec![JsExpr::Var("x".into())])),
        ];
        hoist_super_call(&mut body, None);
        assert!(matches!(&body[1], JsStmt::Expr(JsExpr::SuperCall(_))));
    }

    #[test]
    fn hoist_super_call_rewrites_this_to_prototype() {
        let mut body = vec![JsStmt::Expr(JsExpr::SuperCall(vec![JsExpr::Field {
            object: Box::new(JsExpr::This),
            field: "handler".into(),
        }]))];
        hoist_super_call(&mut body, Some("MyClass"));
        if let JsStmt::Expr(JsExpr::SuperCall(args)) = &body[0] {
            assert!(matches!(&args[0], JsExpr::Field { object, field }
                if field == "handler"
                && matches!(object.as_ref(), JsExpr::Field { object: inner, field: proto }
                    if proto == "prototype"
                    && matches!(inner.as_ref(), JsExpr::Var(n) if n == "MyClass"))));
        } else {
            panic!("expected SuperCall");
        }
    }

    // --- Statement-level rewrites ---

    #[test]
    fn throw_statement_rewrite() {
        let ctx = empty_ctx();
        let stmt = JsStmt::Expr(JsExpr::SystemCall {
            system: "Flash.Exception".into(),
            method: "throw".into(),
            args: vec![JsExpr::Var("err".into())],
        });
        let result = rewrite_stmt(stmt, &ctx);
        assert!(matches!(result, Some(JsStmt::Throw(_))));
    }

    #[test]
    fn standalone_scope_lookup_suppressed() {
        let ctx = empty_ctx();
        let stmt = JsStmt::Expr(scope_lookup("classes:Foo::bar"));
        let result = rewrite_stmt(stmt, &ctx);
        assert!(result.is_none());
    }

    #[test]
    fn set_super_statement_rewrite() {
        let ctx = empty_ctx();
        let stmt = JsStmt::Expr(JsExpr::SystemCall {
            system: "Flash.Class".into(),
            method: "setSuper".into(),
            args: vec![
                JsExpr::This,
                JsExpr::Literal(Constant::String("value".into())),
                JsExpr::Literal(Constant::Int(42)),
            ],
        });
        let result = rewrite_stmt(stmt, &ctx);
        assert!(result.is_some());
        assert!(matches!(&result.unwrap(), JsStmt::Assign {
            target: JsExpr::SuperGet(prop), ..
        } if prop == "value"));
    }

    // --- cachedBind ---

    #[test]
    fn method_ref_in_non_callee_position_bound() {
        let bindable: HashSet<String> = ["update".to_string()].into();
        let mut expr = JsExpr::Field {
            object: Box::new(JsExpr::This),
            field: "update".into(),
        };
        bind_method_refs_expr(&mut expr, &bindable, false);
        assert!(matches!(&expr, JsExpr::Call { callee, args }
            if matches!(callee.as_ref(), JsExpr::Var(n) if n == "cachedBind")
            && args.len() == 2));
    }

    #[test]
    fn method_ref_in_callee_position_not_bound() {
        let bindable: HashSet<String> = ["update".to_string()].into();
        let mut expr = JsExpr::Field {
            object: Box::new(JsExpr::This),
            field: "update".into(),
        };
        bind_method_refs_expr(&mut expr, &bindable, true);
        assert!(matches!(&expr, JsExpr::Field { field, .. } if field == "update"));
    }

    // --- const instance field promotion ---

    #[test]
    fn const_instance_field_resolves_to_class_static() {
        let mut ctx = empty_ctx();
        ctx.has_self = true;
        ctx.class_short_name = Some("MyClass".into());
        ctx.const_instance_fields.insert("MAX_HP".into());

        let expr = JsExpr::Field {
            object: Box::new(JsExpr::This),
            field: "MAX_HP".into(),
        };
        let result = rewrite_expr(expr, &ctx);
        assert!(matches!(&result, JsExpr::Field { object, field }
            if matches!(object.as_ref(), JsExpr::Var(n) if n == "MyClass")
            && field == "MAX_HP"));
    }

    // --- Adversarial / edge cases ---

    #[test]
    fn scope_lookup_cinit_static_field_resolves_to_this() {
        // In cinit context, static fields resolve to this.field, not ClassName.field.
        // This is subtle: cinit runs as the class constructor, so `this` IS the class.
        let mut ctx = empty_ctx();
        ctx.is_cinit = true;
        ctx.static_fields.insert("INSTANCE_COUNT".into());
        let result = resolve_field(
            &scope_lookup("global::INSTANCE_COUNT"),
            "INSTANCE_COUNT",
            &ctx,
        );
        assert!(result.is_some());
        let expr = result.unwrap();
        assert!(
            matches!(&expr, JsExpr::Field { object, field }
                if matches!(object.as_ref(), JsExpr::This) && field == "INSTANCE_COUNT"),
            "cinit should resolve static field to this.field, got {:?}",
            expr
        );
    }

    #[test]
    fn scope_lookup_static_field_owner_from_ancestor() {
        // Static field owned by a different class in the hierarchy.
        let mut ctx = empty_ctx();
        ctx.class_short_name = Some("Child".into());
        ctx.static_field_owners
            .insert("MAX".into(), "Parent".into());
        let result = resolve_field(&scope_lookup("global::MAX"), "MAX", &ctx);
        assert!(result.is_some());
        let expr = result.unwrap();
        assert!(
            matches!(&expr, JsExpr::Field { object, field }
                if matches!(object.as_ref(), JsExpr::Var(n) if n == "Parent") && field == "MAX"),
            "should resolve to Parent.MAX, got {:?}",
            expr
        );
    }

    #[test]
    fn scope_lookup_namespace_stripped_from_field() {
        // Field name like "ns::myField" should strip the namespace prefix.
        let mut ctx = empty_ctx();
        ctx.has_self = true;
        ctx.instance_fields.insert("myField".into());
        let result = resolve_field(&scope_lookup("global::myField"), "ns::myField", &ctx);
        assert!(result.is_some());
        let expr = result.unwrap();
        assert!(
            matches!(&expr, JsExpr::Field { object, field }
                if matches!(object.as_ref(), JsExpr::This) && field == "myField"),
            "should strip namespace and resolve to this.myField, got {:?}",
            expr
        );
    }

    #[test]
    fn scope_call_non_ancestor_with_class_prefix_dispatches_statically() {
        // Call to OtherClass.method — scope arg contains class name, not in ancestors.
        let ctx = empty_ctx();
        let rewritten = resolve_scope_call(
            "doStuff",
            &[JsExpr::Literal(Constant::String(
                "classes:OtherClass::doStuff".into(),
            ))],
            vec![JsExpr::Literal(Constant::Int(1))],
            &ctx,
        );
        // Should produce OtherClass.doStuff(1), NOT bare doStuff(1).
        assert!(
            matches!(&rewritten, JsExpr::Call { callee, .. }
                if matches!(callee.as_ref(), JsExpr::Field { object, field }
                    if matches!(object.as_ref(), JsExpr::Var(n) if n == "OtherClass")
                    && field == "doStuff")),
            "should dispatch to OtherClass.doStuff, got {:?}",
            rewritten
        );
    }

    #[test]
    fn class_coercion_not_triggered_for_multi_arg_call() {
        // ClassName(a, b) with 2 args should NOT be treated as coercion.
        let mut ctx = empty_ctx();
        ctx.known_classes.insert("Sprite".into());
        let rewritten = resolve_scope_call(
            "Sprite",
            &[JsExpr::Literal(Constant::String("Sprite".into()))],
            vec![JsExpr::Var("a".into()), JsExpr::Var("b".into())],
            &ctx,
        );
        // Two args → regular call, NOT a Cast.
        assert!(
            matches!(&rewritten, JsExpr::Call { .. }),
            "multi-arg call should not be coercion, got {:?}",
            rewritten
        );
        assert!(
            !matches!(&rewritten, JsExpr::Cast { .. }),
            "must not produce Cast for multi-arg"
        );
    }

    #[test]
    fn class_coercion_not_triggered_for_non_class_name() {
        // regularFunc(obj) where regularFunc is NOT in known_classes.
        let ctx = empty_ctx();
        let rewritten = resolve_scope_call(
            "regularFunc",
            &[JsExpr::Literal(Constant::String("regularFunc".into()))],
            vec![JsExpr::Var("obj".into())],
            &ctx,
        );
        assert!(
            matches!(&rewritten, JsExpr::Call { .. }),
            "non-class single-arg call should remain a Call, got {:?}",
            rewritten
        );
    }

    #[test]
    fn new_object_duplicate_keys_preserved() {
        // newObject rewrite preserves duplicate keys — dedup_object_keys AST pass
        // handles deduplication with diagnostic warnings.
        let ctx = empty_ctx();
        let args = vec![
            JsExpr::Literal(Constant::String("x".into())),
            JsExpr::Literal(Constant::Int(1)),
            JsExpr::Literal(Constant::String("x".into())),
            JsExpr::Literal(Constant::Int(2)),
        ];
        let result = rewrite_system_call("Flash.Object", "newObject", &args, &ctx);
        assert!(result.is_some());
        if let Some(JsExpr::ObjectInit(pairs)) = result {
            assert_eq!(pairs.len(), 2, "duplicate keys should be preserved");
            assert_eq!(pairs[0].0, "x");
            assert_eq!(pairs[1].0, "x");
            assert!(matches!(&pairs[0].1, JsExpr::Literal(Constant::Int(1))));
            assert!(matches!(&pairs[1].1, JsExpr::Literal(Constant::Int(2))));
        } else {
            panic!("expected ObjectInit");
        }
    }

    #[test]
    fn new_object_odd_arg_count_falls_through() {
        // Odd number of args (3) — not valid key/value pairs, should NOT produce ObjectInit.
        let ctx = empty_ctx();
        let args = vec![
            JsExpr::Literal(Constant::String("a".into())),
            JsExpr::Literal(Constant::Int(1)),
            JsExpr::Literal(Constant::String("orphan".into())),
        ];
        let result = rewrite_system_call("Flash.Object", "newObject", &args, &ctx);
        // Odd count → no rewrite (falls through to None).
        assert!(
            result.is_none(),
            "odd arg count should not produce ObjectInit"
        );
    }

    #[test]
    fn construct_super_with_only_this_produces_shims_only_super() {
        // constructSuper(this) with no additional args → super(_shims)
        let ctx = empty_ctx();
        let args = vec![JsExpr::This];
        let result = rewrite_system_call("Flash.Class", "constructSuper", &args, &ctx);
        assert!(result.is_some());
        if let Some(JsExpr::SuperCall(a)) = result {
            assert_eq!(a.len(), 1, "super() should have exactly _shims arg");
            assert!(
                matches!(&a[0], JsExpr::Var(n) if n == "_shims"),
                "first arg should be _shims"
            );
        } else {
            panic!("expected SuperCall");
        }
    }

    #[test]
    fn hoist_super_already_at_position_zero_is_noop() {
        let mut body = vec![
            JsStmt::Expr(JsExpr::SuperCall(vec![])),
            body_stmt("a"),
            body_stmt("b"),
        ];
        hoist_super_call(&mut body, None);
        // Already at position 0 — should remain there.
        assert!(matches!(&body[0], JsStmt::Expr(JsExpr::SuperCall(_))));
        assert_eq!(body.len(), 3);
    }

    #[test]
    fn hoist_super_no_super_present_is_noop() {
        let mut body = vec![body_stmt("a"), body_stmt("b")];
        let orig_len = body.len();
        hoist_super_call(&mut body, None);
        assert_eq!(body.len(), orig_len);
    }

    #[test]
    fn hoist_super_multiple_deps_hoists_after_last() {
        // super(x, y) depends on both x and y. y is declared after x.
        // Should hoist to just after y's declaration.
        let mut body = vec![
            JsStmt::VarDecl {
                name: "x".into(),
                ty: Some(Type::Int(32)),
                init: Some(JsExpr::Literal(Constant::Int(1))),
                mutable: false,
            },
            body_stmt("unrelated"),
            JsStmt::VarDecl {
                name: "y".into(),
                ty: Some(Type::Int(32)),
                init: Some(JsExpr::Literal(Constant::Int(2))),
                mutable: false,
            },
            body_stmt("also_unrelated"),
            JsStmt::Expr(JsExpr::SuperCall(vec![
                JsExpr::Var("x".into()),
                JsExpr::Var("y".into()),
            ])),
        ];
        hoist_super_call(&mut body, None);
        // Should be at position 3 (after y's decl at position 2).
        assert!(
            matches!(&body[3], JsStmt::Expr(JsExpr::SuperCall(_))),
            "super should be at index 3, got {:?}",
            body[3]
        );
    }

    #[test]
    fn method_bind_inside_call_arg_still_wraps() {
        // foo(this.update) — this.update is NOT in callee position (it's an arg).
        let bindable: HashSet<String> = ["update".to_string()].into();
        let mut expr = JsExpr::Call {
            callee: Box::new(JsExpr::Var("foo".into())),
            args: vec![JsExpr::Field {
                object: Box::new(JsExpr::This),
                field: "update".into(),
            }],
        };
        bind_method_refs_expr(&mut expr, &bindable, false);
        // The arg this.update should be wrapped with cachedBind.
        if let JsExpr::Call { args, .. } = &expr {
            assert!(
                matches!(&args[0], JsExpr::Call { callee, .. }
                    if matches!(callee.as_ref(), JsExpr::Var(n) if n == "cachedBind")),
                "arg should be wrapped with cachedBind, got {:?}",
                args[0]
            );
        } else {
            panic!("expected Call");
        }
    }

    #[test]
    fn method_bind_non_this_field_not_wrapped() {
        // obj.update (not this.update) — should NOT be wrapped.
        let bindable: HashSet<String> = ["update".to_string()].into();
        let mut expr = JsExpr::Field {
            object: Box::new(JsExpr::Var("obj".into())),
            field: "update".into(),
        };
        bind_method_refs_expr(&mut expr, &bindable, false);
        assert!(
            matches!(&expr, JsExpr::Field { object, .. }
                if matches!(object.as_ref(), JsExpr::Var(n) if n == "obj")),
            "non-this field should not be wrapped, got {:?}",
            expr
        );
    }

    #[test]
    fn const_field_not_promoted_without_class_name() {
        // If class_short_name is None, const instance fields should NOT be promoted.
        let mut ctx = empty_ctx();
        ctx.has_self = true;
        ctx.class_short_name = None; // no class name
        ctx.const_instance_fields.insert("MAX_HP".into());

        let expr = JsExpr::Field {
            object: Box::new(JsExpr::This),
            field: "MAX_HP".into(),
        };
        let result = rewrite_expr(expr, &ctx);
        // Without class_short_name, it should remain this.MAX_HP.
        assert!(
            matches!(&result, JsExpr::Field { object, .. }
                if matches!(object.as_ref(), JsExpr::This)),
            "without class name, should stay this.MAX_HP, got {:?}",
            result
        );
    }

    #[test]
    fn unknown_system_call_passes_through() {
        let ctx = empty_ctx();
        let args = vec![JsExpr::Literal(Constant::Int(1))];
        let result = rewrite_system_call("Unknown.System", "mystery", &args, &ctx);
        assert!(result.is_none(), "unknown system calls should return None");
    }

    #[test]
    fn scope_lookup_class_names_mapping_takes_priority() {
        // When a full qualified name matches class_names, it should return
        // the mapped short name, not try other resolution paths.
        let mut ctx = empty_ctx();
        ctx.class_names
            .insert("com.example::LongName".into(), "ShortName".into());
        let result = resolve_field(
            &scope_lookup("global::LongName"),
            "com.example::LongName",
            &ctx,
        );
        assert!(result.is_some());
        let expr = result.unwrap();
        assert!(
            matches!(&expr, JsExpr::Var(n) if n == "ShortName"),
            "should resolve to mapped short name, got {:?}",
            expr
        );
    }
}
