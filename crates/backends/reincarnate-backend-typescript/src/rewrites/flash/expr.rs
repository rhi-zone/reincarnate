//! Expression rewriting.

use reincarnate_core::ir::{CastKind, Constant, Type};

use crate::js_ast::JsExpr;
use crate::runtime::SYSTEM_NAMES;

use super::context::FlashRewriteCtx;
use super::method_bind::extract_object_key;
use super::scope::{
    is_scope_lookup, resolve_field, resolve_scope_call, resolve_scope_lookup, scope_lookup_args,
};
use super::stmt::rewrite_stmts;
use super::super_hoist::subst_var_to_this_stmts;

pub(super) fn rewrite_exprs(exprs: Vec<JsExpr>, ctx: &FlashRewriteCtx) -> Vec<JsExpr> {
    exprs.into_iter().map(|e| rewrite_expr(e, ctx)).collect()
}

/// Rewrite a single JsExpr, resolving Flash SystemCalls and scope lookups.
///
/// Matches compound patterns (scope-lookup embedded in Field/Call/Binary)
/// TOP-DOWN before recursing into children.
pub(super) fn rewrite_expr(expr: JsExpr, ctx: &FlashRewriteCtx) -> JsExpr {
    // --- Top-down compound pattern matching ---

    // Field { object: scope_lookup, field } → resolved field access
    if let JsExpr::Field {
        ref object,
        ref field,
    } = expr
    {
        if is_scope_lookup(object) {
            if let Some(resolved) = resolve_field(object, field, ctx) {
                return resolved;
            }
        }
    }

    // Call { callee: Field(scope_lookup, method), args } → resolved scope call
    if let JsExpr::Call {
        ref callee,
        ref args,
    } = expr
    {
        if let JsExpr::Field {
            ref object,
            ref field,
        } = **callee
        {
            if let Some(scope_args) = scope_lookup_args(object) {
                let rewritten_args = rewrite_exprs(args.clone(), ctx);
                return resolve_scope_call(field, scope_args, rewritten_args, ctx);
            }
        }
    }

    // AVM2 `Op::Call` on `this` in a static method = type coercion.
    // Bytecode: getlocal0; getlocal0; <arg>; call 2  → this(this, arg)
    // In a static method, `this` IS the class, so `this(arg)` = `ClassName(arg)`
    // = coerce arg to ClassName → asType(arg, ClassName).
    if let JsExpr::Call {
        ref callee,
        ref args,
    } = expr
    {
        if matches!(**callee, JsExpr::This) && !args.is_empty() && matches!(&args[0], JsExpr::This)
        {
            let rest = rewrite_exprs(args[1..].to_vec(), ctx);
            if rest.len() == 1 {
                if let Some(ref class_name) = ctx.class_short_name {
                    let ty = ctx
                        .class_type_ids
                        .get(class_name.as_str())
                        .copied()
                        .map(Type::Instance)
                        .unwrap_or(Type::Value);
                    return JsExpr::Cast {
                        expr: Box::new(rest.into_iter().next().unwrap()),
                        ty,
                        kind: CastKind::NullableCoerce,
                    };
                }
            }
        }
    }

    // Binary { lhs: scope_lookup, rhs } → strip scope lookup, return other side
    if let JsExpr::Binary {
        ref lhs, ref rhs, ..
    } = expr
    {
        if is_scope_lookup(lhs) {
            return rewrite_expr(rhs.as_ref().clone(), ctx);
        }
        if is_scope_lookup(rhs) {
            return rewrite_expr(lhs.as_ref().clone(), ctx);
        }
    }

    // --- SystemCall rewrites ---
    if let JsExpr::SystemCall {
        ref system,
        ref method,
        ref args,
    } = expr
    {
        if let Some(result) = rewrite_system_call(system, method, args, ctx) {
            return result;
        }
    }

    // AS3 XML/XMLList.length() is a method; TypeScript's Array.length is a property.
    // Rewrite `expr.length()` (no args) → `expr.length` so TS2349 doesn't fire.
    if let JsExpr::Call {
        ref callee,
        ref args,
    } = expr
    {
        if args.is_empty() {
            if let JsExpr::Field {
                ref field,
                ref object,
            } = **callee
            {
                if field == "length" {
                    let object = Box::new(rewrite_expr(*object.clone(), ctx));
                    return JsExpr::Field {
                        object,
                        field: "length".to_string(),
                    };
                }
            }
        }
    }

    // AS3 `String.replace(pattern, repl)` auto-coerces `repl` to String.
    // TypeScript's String.replace() requires `string` — wrap with String() to avoid TS2769.
    if let JsExpr::Call {
        ref callee,
        ref args,
    } = expr
    {
        if args.len() == 2 {
            if let JsExpr::Field { ref field, .. } = **callee {
                if field == "replace" {
                    if let JsExpr::Call { callee, args } = expr {
                        let callee_rewritten = Box::new(rewrite_expr(*callee, ctx));
                        let mut args_iter = args.into_iter();
                        let search = rewrite_expr(args_iter.next().unwrap(), ctx);
                        let repl = rewrite_expr(args_iter.next().unwrap(), ctx);
                        // Wrap replacement with String() unless already a string literal
                        let repl = if matches!(&repl, JsExpr::Literal(Constant::String(_)))
                            || matches!(&repl, JsExpr::Call { callee, .. } if matches!(**callee, JsExpr::Var(ref n) if n == "String"))
                        {
                            repl
                        } else {
                            JsExpr::Call {
                                callee: Box::new(JsExpr::Var("String".to_string())),
                                args: vec![repl],
                            }
                        };
                        return JsExpr::Call {
                            callee: callee_rewritten,
                            args: vec![search, repl],
                        };
                    }
                }
            }
        }
    }

    // `.apply(receiver, args_array)` — cast args_array as `any` to satisfy TS strict
    // Function.apply() typing (TS2345: any[] not assignable to exact param tuples).
    if let JsExpr::Call {
        ref callee,
        ref args,
    } = expr
    {
        if args.len() == 2 {
            if let JsExpr::Field { ref field, .. } = **callee {
                if field == "apply" {
                    if let JsExpr::Call { callee, args } = expr {
                        let callee_rewritten = Box::new(rewrite_expr(*callee, ctx));
                        let mut args_iter = args.into_iter();
                        let receiver = rewrite_expr(args_iter.next().unwrap(), ctx);
                        let args_array = rewrite_expr(args_iter.next().unwrap(), ctx);
                        return JsExpr::Call {
                            callee: callee_rewritten,
                            args: vec![
                                receiver,
                                JsExpr::Cast {
                                    expr: Box::new(args_array),
                                    ty: Type::Value,
                                    kind: CastKind::NullableCoerce,
                                },
                            ],
                        };
                    }
                }
            }
        }
    }

    // `regex.exec(str)[i]` → `regex.exec(str)![i]` — RegExp.exec() returns
    // RegExpExecArray | null in TypeScript; add non-null assertion when the result
    // is immediately indexed (the source AS3 has no null check either).
    if let JsExpr::Index { ref collection, .. } = expr {
        if let JsExpr::Call { ref callee, .. } = **collection {
            if let JsExpr::Field { ref field, .. } = **callee {
                if field == "exec" {
                    if let JsExpr::Index { collection, index } = expr {
                        let collection = rewrite_expr(*collection, ctx);
                        let index = rewrite_expr(*index, ctx);
                        return JsExpr::Index {
                            collection: Box::new(JsExpr::NonNull(Box::new(collection))),
                            index: Box::new(index),
                        };
                    }
                }
            }
        }
    }

    // --- Recurse into children ---
    match expr {
        JsExpr::Literal(_)
        | JsExpr::Var(_)
        | JsExpr::Regex(_)
        | JsExpr::This
        | JsExpr::Activation => expr,

        JsExpr::Binary { op, lhs, rhs } => JsExpr::Binary {
            op,
            lhs: Box::new(rewrite_expr(*lhs, ctx)),
            rhs: Box::new(rewrite_expr(*rhs, ctx)),
        },

        JsExpr::NonNull(inner) => JsExpr::NonNull(Box::new(rewrite_expr(*inner, ctx))),

        JsExpr::Unary { op, expr: inner } => JsExpr::Unary {
            op,
            expr: Box::new(rewrite_expr(*inner, ctx)),
        },

        JsExpr::Cmp { kind, lhs, rhs } => JsExpr::Cmp {
            kind,
            lhs: Box::new(rewrite_expr(*lhs, ctx)),
            rhs: Box::new(rewrite_expr(*rhs, ctx)),
        },

        JsExpr::Field { object, field } => {
            let object = Box::new(rewrite_expr(*object, ctx));
            // Rewrite this.CONST → ClassName.CONST for promoted instance Const fields.
            // Also rewrite this.STATIC → ClassName.STATIC in constructors (TS2576).
            if matches!(*object, JsExpr::This) {
                if let Some(ref class_name) = ctx.class_short_name {
                    if ctx.const_instance_fields.contains(&field)
                        || (ctx.is_constructor && ctx.static_fields.contains(&field))
                    {
                        return JsExpr::Field {
                            object: Box::new(JsExpr::Var(class_name.clone())),
                            field,
                        };
                    }
                }
            }
            // Rewrite anyExpr.UNIQUE_STATIC_FIELD → OwnerClass.UNIQUE_STATIC_FIELD (TS2576).
            //
            // Skip when the object is already an appropriate class reference:
            // - Var(owner) — already the right class
            // - Var(name) where name is a known AS3 class or external type (e.g. another game
            //   class that happens to have the same field name, or an external type like Sprite)
            // - Var(name) starting with uppercase — JS globals like Math, Object, String, etc.
            //
            // DO apply when object is `This` — AS3 allows `this.STATIC_FIELD` in instance
            // methods but TypeScript doesn't (TS2576); rewrite to OwnerClass.STATIC_FIELD.
            if let Some(owner) = ctx.unique_static_fields.get(&field) {
                let skip = match &*object {
                    JsExpr::Var(n) if n == owner => true,
                    JsExpr::Var(n) if ctx.known_classes.contains(n.as_str()) => true,
                    JsExpr::Var(n) if n.starts_with(|c: char| c.is_uppercase()) => true,
                    _ => false,
                };
                if !skip {
                    return JsExpr::Field {
                        object: Box::new(JsExpr::Var(owner.clone())),
                        field,
                    };
                }
            }
            JsExpr::Field { object, field }
        }

        JsExpr::Index { collection, index } => JsExpr::Index {
            collection: Box::new(rewrite_expr(*collection, ctx)),
            index: Box::new(rewrite_expr(*index, ctx)),
        },

        JsExpr::Call { callee, args } => {
            let callee = rewrite_expr(*callee, ctx);
            let mut args = rewrite_exprs(args, ctx);
            // AS3 `Op::Call` passes the activation scope (outer `this`) as the
            // first argument.  When the callee was a `newFunction` closure and is
            // now an arrow function, the scope is already captured lexically —
            // the extra `this` arg no longer matches any parameter.  Strip it so
            // the IIFE call `})(this, actual_arg)` becomes `})(actual_arg)`.
            if let JsExpr::ArrowFunction { ref params, .. } = callee {
                if args.len() == params.len() + 1
                    && matches!(
                        args.first(),
                        Some(JsExpr::This) | Some(JsExpr::Field { .. })
                    )
                {
                    args.remove(0);
                }
            }
            JsExpr::Call {
                callee: Box::new(callee),
                args,
            }
        }

        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => JsExpr::Ternary {
            cond: Box::new(rewrite_expr(*cond, ctx)),
            then_val: Box::new(rewrite_expr(*then_val, ctx)),
            else_val: Box::new(rewrite_expr(*else_val, ctx)),
        },

        JsExpr::LogicalOr { lhs, rhs } => JsExpr::LogicalOr {
            lhs: Box::new(rewrite_expr(*lhs, ctx)),
            rhs: Box::new(rewrite_expr(*rhs, ctx)),
        },

        JsExpr::LogicalAnd { lhs, rhs } => JsExpr::LogicalAnd {
            lhs: Box::new(rewrite_expr(*lhs, ctx)),
            rhs: Box::new(rewrite_expr(*rhs, ctx)),
        },

        JsExpr::Cast {
            expr: inner,
            ty,
            kind,
        } => JsExpr::Cast {
            expr: Box::new(rewrite_expr(*inner, ctx)),
            ty,
            kind,
        },

        JsExpr::TypeCheck {
            expr: inner,
            ty,
            use_instanceof,
        } => {
            // Disambiguate Instance type names using class_type_ids so that
            // `isType(x, GooArmor)` becomes `isType(x, Armors_GooArmor)` when two
            // classes share the same short name.
            // The ts_name is stored as an alias in class_type_ids — the TypeId
            // carries the disambiguated name through to print time via MODULE_TYPES.
            // Instance(TypeId) — name resolution happens at print time via MODULE_TYPES
            JsExpr::TypeCheck {
                expr: Box::new(rewrite_expr(*inner, ctx)),
                ty,
                use_instanceof,
            }
        }

        JsExpr::ArrayInit(elems) => JsExpr::ArrayInit(rewrite_exprs(elems, ctx)),
        JsExpr::ObjectInit(pairs) => JsExpr::ObjectInit(
            pairs
                .into_iter()
                .map(|(k, v)| (k, rewrite_expr(v, ctx)))
                .collect(),
        ),
        JsExpr::TupleInit(elems) => JsExpr::TupleInit(rewrite_exprs(elems, ctx)),

        JsExpr::Not(inner) => JsExpr::Not(Box::new(rewrite_expr(*inner, ctx))),
        JsExpr::PostIncrement(inner) => JsExpr::PostIncrement(Box::new(rewrite_expr(*inner, ctx))),
        JsExpr::Spread(inner) => JsExpr::Spread(Box::new(rewrite_expr(*inner, ctx))),

        JsExpr::GeneratorCreate { func, args } => JsExpr::GeneratorCreate {
            func,
            args: rewrite_exprs(args, ctx),
        },
        JsExpr::GeneratorResume(inner) => {
            JsExpr::GeneratorResume(Box::new(rewrite_expr(*inner, ctx)))
        }
        JsExpr::Yield(v) => JsExpr::Yield(v.map(|e| Box::new(rewrite_expr(*e, ctx)))),

        JsExpr::New { callee, args } => JsExpr::New {
            callee: Box::new(rewrite_expr(*callee, ctx)),
            args: rewrite_exprs(args, ctx),
        },
        JsExpr::TypeOf(inner) => JsExpr::TypeOf(Box::new(rewrite_expr(*inner, ctx))),
        JsExpr::In { key, object } => JsExpr::In {
            key: Box::new(rewrite_expr(*key, ctx)),
            object: Box::new(rewrite_expr(*object, ctx)),
        },
        JsExpr::Delete { object, key } => JsExpr::Delete {
            object: Box::new(rewrite_expr(*object, ctx)),
            key: Box::new(rewrite_expr(*key, ctx)),
        },
        JsExpr::SuperCall(args) => JsExpr::SuperCall(rewrite_exprs(args, ctx)),
        JsExpr::SuperMethodCall { method, args } => JsExpr::SuperMethodCall {
            method,
            args: rewrite_exprs(args, ctx),
        },
        JsExpr::SuperGet(_) => expr,
        JsExpr::SuperSet { prop, value } => JsExpr::SuperSet {
            prop,
            value: Box::new(rewrite_expr(*value, ctx)),
        },

        JsExpr::SystemCall {
            system,
            method,
            args,
        } => JsExpr::SystemCall {
            system,
            method,
            args: rewrite_exprs(args, ctx),
        },

        JsExpr::ArrowFunction {
            params,
            return_ty,
            body,
            has_rest_param,
            cast_as,
            infer_param_types,
        } => JsExpr::ArrowFunction {
            params,
            return_ty,
            body: rewrite_stmts(body, ctx),
            has_rest_param,
            cast_as,
            infer_param_types,
        },

        // NullCoalesceAssign is a GML-only construct emitted by the GML rewrite pass.
        // Flash never emits it — pass through unchanged.
        JsExpr::NullCoalesceAssign { target, value } => JsExpr::NullCoalesceAssign {
            target: Box::new(rewrite_expr(*target, ctx)),
            value: Box::new(rewrite_expr(*value, ctx)),
        },

        // Assign is a GML-only construct (array_resize_arr); pass through unchanged for Flash.
        JsExpr::Assign { lhs, rhs } => JsExpr::Assign {
            lhs: Box::new(rewrite_expr(*lhs, ctx)),
            rhs: Box::new(rewrite_expr(*rhs, ctx)),
        },

        // LooseEq/LooseNe are SugarCube-only; pass through unchanged for Flash.
        JsExpr::LooseEq { lhs, rhs } => JsExpr::LooseEq {
            lhs: Box::new(rewrite_expr(*lhs, ctx)),
            rhs: Box::new(rewrite_expr(*rhs, ctx)),
        },
        JsExpr::LooseNe { lhs, rhs } => JsExpr::LooseNe {
            lhs: Box::new(rewrite_expr(*lhs, ctx)),
            rhs: Box::new(rewrite_expr(*rhs, ctx)),
        },
    }
}

// ---------------------------------------------------------------------------
// SystemCall expression rewrites
// ---------------------------------------------------------------------------

/// Rewrite a Flash SystemCall node in the JsExpr tree.
///
/// Returns `Some(JsExpr)` if the call was recognized, `None` for unmapped.
pub(super) fn rewrite_system_call(
    system: &str,
    method: &str,
    args: &[JsExpr],
    ctx: &FlashRewriteCtx,
) -> Option<JsExpr> {
    // Generic shim system calls → this._shims.{system}.{method}(args).
    // Only applies in instance context (has_self) that is not a static method;
    // static methods don't have a `this._shims` and fall through to SystemCall.
    if SYSTEM_NAMES.contains(&system) && ctx.has_self && !ctx.is_static {
        let rewritten_args = rewrite_exprs(args.to_vec(), ctx);
        return Some(JsExpr::Call {
            callee: Box::new(JsExpr::Field {
                object: Box::new(JsExpr::Field {
                    object: Box::new(JsExpr::Field {
                        object: Box::new(JsExpr::This),
                        field: "_shims".to_string(),
                    }),
                    field: system.to_string(),
                }),
                field: method.to_string(),
            }),
            args: rewritten_args,
        });
    }

    // Flash.Memory (Alchemy domain memory) → this._shims.memory.{method}(args).
    // Per-instance heap: each FlashShims carries its own FlashMemory instance.
    if system == "Flash.Memory" && ctx.has_self && !ctx.is_static {
        let rewritten_args = rewrite_exprs(args.to_vec(), ctx);
        return Some(JsExpr::Call {
            callee: Box::new(JsExpr::Field {
                object: Box::new(JsExpr::Field {
                    object: Box::new(JsExpr::Field {
                        object: Box::new(JsExpr::This),
                        field: "_shims".to_string(),
                    }),
                    field: "memory".to_string(),
                }),
                field: method.to_string(),
            }),
            args: rewritten_args,
        });
    }

    // constructSuper → super(_shims, ...args), super(...args), or void 0
    if system == "Flash.Class" && method == "constructSuper" {
        if ctx.suppress_super {
            return Some(JsExpr::Literal(Constant::Null));
        }
        let rest: Vec<JsExpr> = rewrite_exprs(args.iter().skip(1).cloned().collect(), ctx);
        if ctx.parent_is_runtime {
            return Some(JsExpr::SuperCall(rest));
        }
        let mut super_args = vec![JsExpr::Var("_shims".to_string())];
        super_args.extend(rest);
        return Some(JsExpr::SuperCall(super_args));
    }

    // newFunction → inline arrow function (or this.methodRef fallback)
    if system == "Flash.Object" && method == "newFunction" && args.len() == 1 {
        if let JsExpr::Literal(Constant::String(ref name)) = args[0] {
            let short = name.rsplit("::").next().unwrap_or(name);
            if let Some(closure_func) = ctx.closure_bodies.get(short).cloned() {
                let rewritten = super::rewrite_flash_function(closure_func, ctx);
                // Skip first param (activation scope object).
                let params = if rewritten.params.len() > 1 {
                    rewritten.params[1..].to_vec()
                } else {
                    vec![]
                };
                let mut body = rewritten.body;
                if ctx.has_self && !ctx.is_static {
                    // Closures are compiled with self_param_name=None (their first
                    // param is the activation scope, not `this`).  If the closure
                    // body references the outer method's self parameter (`v0`) it
                    // remains as Var("v0").  Arrow functions inherit `this` from the
                    // enclosing scope, so Var("v0") → This is correct.
                    subst_var_to_this_stmts(&mut body, "v0");
                }
                return Some(JsExpr::ArrowFunction {
                    params,
                    return_ty: rewritten.return_ty,
                    body,
                    has_rest_param: rewritten.has_rest_param,
                    cast_as: None,
                    infer_param_types: false,
                });
            }
            // Fallback: non-compiled closure → this.$closureN
            return Some(JsExpr::Field {
                object: Box::new(JsExpr::This),
                field: short.to_string(),
            });
        }
    }

    // applyType(base, ...typeArgs) → Array
    // Vector is the only parameterized type in AS3; generics are erased in TS.
    if system == "Flash.Object" && method == "applyType" {
        return Some(JsExpr::Var("Array".to_string()));
    }

    // construct → new Ctor(this._shims, args)  (but `new Object()` → `{}`)
    if system == "Flash.Object" && method == "construct" && !args.is_empty() {
        let callee = rewrite_expr(args[0].clone(), ctx);
        let rest = rewrite_exprs(args[1..].to_vec(), ctx);
        // `new Object()` with no constructor args → empty object literal
        if rest.is_empty() && matches!(&callee, JsExpr::Var(name) if name == "Object") {
            return Some(JsExpr::ObjectInit(Vec::new()));
        }
        // Only inject this._shims for user-defined classes (those in the SWF module).
        // Native JS and Flash runtime classes (Error, Array, Event, Font, etc.) don't
        // accept _shims and will produce TS2554 if it is passed.
        // Unknown callees (computed index, `new this()`) are treated as user classes —
        // they always refer to SWF-defined symbols (e.g. embedded frame classes, singletons).
        let is_user_class = match &callee {
            JsExpr::Var(name) => ctx.class_names.values().any(|s| s == name),
            // Unknown callee: constructed from an array index or `this` reference.
            // These always refer to user-defined classes — embedded frame symbols or
            // the current class itself (singleton `getInstance()` pattern).
            // In instance context: inject this._shims.
            // In static context: inject null as any (placeholder — no this available).
            JsExpr::Index { .. } | JsExpr::This => true,
            _ => false,
        };
        // Inject this._shims as first arg so the new instance inherits the
        // current game's shim set.
        // In static context (no `this`), inject `null as any` as a shims placeholder.
        // Static factory methods (e.g. ConsumableLib.mk, StatusAffects.mk) create
        // data-class instances during cinit; those objects typically never call
        // shim-dependent methods at runtime, so null is safe.
        let mut new_args = if !is_user_class {
            vec![]
        } else if ctx.is_static || ctx.is_cinit {
            vec![JsExpr::Cast {
                expr: Box::new(JsExpr::Literal(Constant::Null)),
                ty: Type::Value,
                kind: CastKind::NullableCoerce,
            }]
        } else {
            vec![JsExpr::Field {
                object: Box::new(JsExpr::This),
                field: "_shims".to_string(),
            }]
        };
        new_args.extend(rest);
        let new_expr = JsExpr::New {
            callee: Box::new(callee.clone()),
            args: new_args,
        };
        // AS3 XML/XMLList have implicit string coercion (toString() on assignment).
        // TypeScript's XML class has no such coercion; wrap with `as any` so the
        // constructed value is assignable to string-typed fields (avoids TS2322).
        if matches!(&callee, JsExpr::Var(n) if matches!(n.as_str(), "XML" | "XMLList")) {
            return Some(JsExpr::Cast {
                expr: Box::new(new_expr),
                ty: Type::Value,
                kind: CastKind::NullableCoerce,
            });
        }
        return Some(new_expr);
    }

    // findPropStrict/findProperty → scope resolution
    if system == "Flash.Scope" && (method == "findPropStrict" || method == "findProperty") {
        return Some(match resolve_scope_lookup(args, ctx) {
            super::context::ScopeResolution::Ancestor(ref class_name) => {
                if ctx.is_cinit {
                    JsExpr::This
                } else {
                    JsExpr::Var(class_name.clone())
                }
            }
            super::context::ScopeResolution::ScopeLookup => {
                // Standalone scope lookup — emit empty var (will be filtered at stmt level).
                JsExpr::Var(String::new())
            }
        });
    }

    // newActivation → ({}) for functions that still use the activation object
    // (those with closures that need scope-chain access).
    if system == "Flash.Scope" && method == "newActivation" && args.is_empty() {
        return Some(JsExpr::Activation);
    }

    // typeOf → typeof expr
    if system == "Flash.Object" && method == "typeOf" && args.len() == 1 {
        return Some(JsExpr::TypeOf(Box::new(rewrite_expr(args[0].clone(), ctx))));
    }

    // hasProperty(obj, k) → k in obj
    if system == "Flash.Object" && method == "hasProperty" && args.len() == 2 {
        return Some(JsExpr::In {
            key: Box::new(rewrite_expr(args[1].clone(), ctx)),
            object: Box::new(rewrite_expr(args[0].clone(), ctx)),
        });
    }

    // deleteProperty(obj, k) → delete obj[k]
    if system == "Flash.Object" && method == "deleteProperty" && args.len() == 2 {
        return Some(JsExpr::Delete {
            object: Box::new(rewrite_expr(args[0].clone(), ctx)),
            key: Box::new(rewrite_expr(args[1].clone(), ctx)),
        });
    }

    // newObject(k1, v1, k2, v2, ...) → { k1: v1, k2: v2, ... }
    // Duplicate keys are left in place — the `dedup_object_keys` AST pass
    // handles deduplication with proper diagnostic warnings.
    if system == "Flash.Object" && method == "newObject" {
        if args.is_empty() {
            return Some(JsExpr::ObjectInit(Vec::new()));
        }
        if args.len().is_multiple_of(2) {
            let pairs: Vec<_> = args
                .chunks_exact(2)
                .map(|pair| {
                    let key = extract_object_key(&pair[0]);
                    let val = rewrite_expr(pair[1].clone(), ctx);
                    (key, val)
                })
                .collect();
            return Some(JsExpr::ObjectInit(pairs));
        }
    }

    // callSuper(this, "method", ...args) → super.method(args)
    if system == "Flash.Class" && method == "callSuper" && args.len() >= 2 {
        if let JsExpr::Literal(Constant::String(ref name)) = args[1] {
            let short = name.rsplit("::").next().unwrap_or(name);
            let rest = rewrite_exprs(args[2..].to_vec(), ctx);
            return Some(JsExpr::SuperMethodCall {
                method: short.to_string(),
                args: rest,
            });
        }
    }

    // getSuper(this, "prop") → super.prop
    if system == "Flash.Class" && method == "getSuper" && args.len() == 2 {
        if let JsExpr::Literal(Constant::String(ref name)) = args[1] {
            let short = name.rsplit("::").next().unwrap_or(name);
            return Some(JsExpr::SuperGet(short.to_string()));
        }
    }

    // setSuper(this, "prop", value) → (super.prop = value)
    if system == "Flash.Class" && method == "setSuper" && args.len() == 3 {
        if let JsExpr::Literal(Constant::String(ref name)) = args[1] {
            let short = name.rsplit("::").next().unwrap_or(name);
            return Some(JsExpr::SuperSet {
                prop: short.to_string(),
                value: Box::new(rewrite_expr(args[2].clone(), ctx)),
            });
        }
    }

    None
}
