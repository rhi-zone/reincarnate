//! `ValidateNoEscapedTypeVars` validation pass.
//!
//! Enforces the invariant that `Type::InferVar` must never persist in
//! `func.value_types`, `func.sig.params`, `func.sig.return_ty`, or any
//! `TypeDecl::Object` field type in `module.types` after inference completes.
//!
//! An `InferVar` surviving past the inference phase means the solver failed to
//! resolve a type variable to a concrete type or `Unknown` — it is an inference
//! bug, not a legitimate type.  This pass surfaces those bugs as hard-error
//! diagnostics so they are visible and actionable rather than silently emitting
//! `unknown` in the TypeScript backend.
//!
//! This pass runs after all inference and dead-code-elimination passes.
//! Running after DCE avoids flagging dead values that were never emitted.
//!
//! **Source-location note:** like `ValidateCalledStubs`, precise per-value
//! source locations are not available at this stage; diagnostics use the
//! function or type name with `line: 0, col: 0`.  See TODO.md for the
//! source-location-granularity follow-up task.

use std::collections::HashSet;

use crate::error::CoreError;
use crate::ir::func::FuncId;
use crate::ir::module::TypeDecl;
use crate::ir::ty::Type;
use crate::ir::Module;
use crate::pipeline::checker::{Diagnostic, DiagnosticCode, RcDiagnostic, Severity};
use crate::pipeline::{Transform, TransformResult};

// ---------------------------------------------------------------------------
// contains_nonpersistable_typevar
// ---------------------------------------------------------------------------

/// Returns `true` if `ty` contains a `Type::InferVar` anywhere in its tree.
///
/// Modeled on `is_concrete` in `constraint_collect.rs` — exhaustive match,
/// no `_` wildcard, so future `Type` variants force a decision at this site.
///
/// When `Type::Template` is added, add an arm here:
///
/// ```text
/// // FUTURE: Type::Template(u32) must also never persist in value_types.
/// // Add the arm below and update the doc-comment when the variant lands.
/// // Type::Template(_) => true,
/// ```
fn contains_nonpersistable_typevar(ty: &Type) -> bool {
    match ty {
        // The target: any InferVar anywhere in the tree is a leak.
        Type::InferVar(_) => true,

        // Compound types — recurse into inner types.
        Type::Array(elem) => contains_nonpersistable_typevar(elem),
        Type::Map(k, v) => contains_nonpersistable_typevar(k) || contains_nonpersistable_typevar(v),
        Type::Option(inner) => contains_nonpersistable_typevar(inner),
        Type::Tuple(elems) => elems.iter().any(contains_nonpersistable_typevar),
        Type::Union(variants) => variants.iter().any(contains_nonpersistable_typevar),
        Type::Function(sig) => {
            sig.params.iter().any(contains_nonpersistable_typevar)
                || contains_nonpersistable_typevar(&sig.return_ty)
        }
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => {
            contains_nonpersistable_typevar(yield_ty) || contains_nonpersistable_typevar(return_ty)
        }

        // Leaf types — no nested type variables, cannot contain InferVar.
        Type::Void
        | Type::Bool
        | Type::Int(_)
        | Type::UInt(_)
        | Type::Float(_)
        | Type::String
        | Type::Instance(_)
        | Type::ClassRef(_)
        | Type::Value => false,
    }
}

// ---------------------------------------------------------------------------
// ValidateNoEscapedTypeVars
// ---------------------------------------------------------------------------

/// `ValidateNoEscapedTypeVars` — hard-error on any `Type::InferVar` that
/// persists in the module after inference completes.
pub struct ValidateNoEscapedTypeVars;

impl Transform for ValidateNoEscapedTypeVars {
    fn name(&self) -> &str {
        "validate-no-escaped-type-vars"
    }

    fn run_once(&self) -> bool {
        true
    }

    /// Requires at least: all inference passes (ConstraintSolveHM runs twice —
    /// once before and once after mem2reg; requiring it here places this pass
    /// after the second run) and DCE (so dead values don't produce spurious
    /// diagnostics).  Mirrors the convention of `ValidateCalledStubs`.
    fn requires(&self) -> &[&str] {
        &["constraint-solve-hm", "dead-code-elimination"]
    }

    fn apply(
        &self,
        mut module: Module,
        _dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        // --- Walk all functions -------------------------------------------
        for (_func_id, func) in module.functions.iter() {
            let func_name = func.name.clone();

            // value_types: every SSA value's recorded type.
            for (_vid, ty) in func.value_types.iter() {
                if contains_nonpersistable_typevar(ty) {
                    module.diagnostics.push(Diagnostic {
                        file: func_name.clone(),
                        line: 0,
                        col: 0,
                        code: DiagnosticCode::Rc(RcDiagnostic::EscapedTypeVar),
                        severity: Severity::Error,
                        message: format!(
                            "InferVar escaped inference in `{func_name}` value_types: \
                             type variable was not resolved — this is an inference bug"
                        ),
                    });
                    // One diagnostic per function is enough to surface the issue;
                    // additional value hits in the same function are redundant noise.
                    break;
                }
            }

            // sig.params: function parameter types.
            for param_ty in &func.sig.params {
                if contains_nonpersistable_typevar(param_ty) {
                    module.diagnostics.push(Diagnostic {
                        file: func_name.clone(),
                        line: 0,
                        col: 0,
                        code: DiagnosticCode::Rc(RcDiagnostic::EscapedTypeVar),
                        severity: Severity::Error,
                        message: format!(
                            "InferVar escaped inference in `{func_name}` sig.params: \
                             parameter type was not resolved — this is an inference bug"
                        ),
                    });
                    break;
                }
            }

            // sig.return_ty: function return type.
            if contains_nonpersistable_typevar(&func.sig.return_ty) {
                module.diagnostics.push(Diagnostic {
                    file: func_name.clone(),
                    line: 0,
                    col: 0,
                    code: DiagnosticCode::Rc(RcDiagnostic::EscapedTypeVar),
                    severity: Severity::Error,
                    message: format!(
                        "InferVar escaped inference in `{func_name}` sig.return_ty: \
                         return type was not resolved — this is an inference bug"
                    ),
                });
            }
        }

        // --- Walk all type declarations -----------------------------------
        for (_type_id, type_decl) in module.types.iter() {
            if let TypeDecl::Object { name, fields, .. } = type_decl {
                let type_name = name.as_deref().unwrap_or("<anonymous>");
                for field in fields {
                    if contains_nonpersistable_typevar(&field.ty) {
                        module.diagnostics.push(Diagnostic {
                            file: type_name.to_string(),
                            line: 0,
                            col: 0,
                            code: DiagnosticCode::Rc(RcDiagnostic::EscapedTypeVar),
                            severity: Severity::Error,
                            message: format!(
                                "InferVar escaped inference in type `{type_name}` \
                                 field `{}`: field type was not resolved — this is an inference bug",
                                field.name
                            ),
                        });
                    }
                }
            }
        }

        Ok(TransformResult {
            module,
            changed: false,
            changed_funcs: HashSet::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityRef;
    use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
    use crate::ir::module::{FieldDef, TypeDecl};
    use crate::ir::ty::{FunctionSig, TypeVarId};
    use crate::ir::{Type, Visibility};
    use crate::pipeline::checker::{DiagnosticCode, RcDiagnostic};
    use crate::pipeline::Transform;

    fn infer_var() -> Type {
        Type::InferVar(TypeVarId::new(0))
    }

    /// (1) A Module with an InferVar left in value_types → Error diagnostic produced.
    #[test]
    fn infervar_in_value_types_produces_diagnostic() {
        // Build a function with an Unknown return type so we can manually
        // inject an InferVar into value_types after the fact.
        let sig = FunctionSig {
            params: vec![Type::Float(64)],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("leaky_fn", sig, Visibility::Private);
        let p = fb.param(0);
        fb.ret(Some(p));
        let mut func = fb.build();

        // Inject an InferVar into the first value's type.
        if let Some(vid) = func.value_types.keys().next() {
            func.value_types[vid] = infer_var();
        }

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let result = ValidateNoEscapedTypeVars.apply(module, None).unwrap();
        let escaped_diags: Vec<_> = result
            .module
            .diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::Rc(RcDiagnostic::EscapedTypeVar))
            .collect();
        assert!(
            !escaped_diags.is_empty(),
            "expected EscapedTypeVar diagnostic for InferVar in value_types"
        );
        assert_eq!(escaped_diags[0].severity, Severity::Error);
        assert!(!result.changed);
    }

    /// (2) A clean Module with no InferVars → no EscapedTypeVar diagnostic.
    #[test]
    fn clean_module_no_diagnostic() {
        let sig = FunctionSig {
            params: vec![Type::Float(64)],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("clean_fn", sig, Visibility::Private);
        let p = fb.param(0);
        fb.ret(Some(p));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();

        let result = ValidateNoEscapedTypeVars.apply(module, None).unwrap();
        let escaped_diags: Vec<_> = result
            .module
            .diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::Rc(RcDiagnostic::EscapedTypeVar))
            .collect();
        assert!(
            escaped_diags.is_empty(),
            "expected no EscapedTypeVar diagnostics for clean module"
        );
        assert!(!result.changed);
    }

    /// (3) InferVar in sig.return_ty and in a TypeDecl::Object field → both detected.
    #[test]
    fn infervar_in_return_ty_and_type_field_both_detected() {
        // Function with InferVar as its return type.
        let sig = FunctionSig {
            params: vec![],
            return_ty: infer_var(),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("leaky_return", sig, Visibility::Private);
        fb.ret(None);

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());

        // TypeDecl::Object with an InferVar field type.
        mb.module_mut().types.push(TypeDecl::Object {
            name: Some("LeakyStruct".to_string()),
            namespace: vec![],
            visibility: Visibility::Public,
            parent: None,
            fields: vec![FieldDef {
                name: "bad_field".to_string(),
                ty: infer_var(),
                default: None,
            }],
            methods: vec![],
            class_ref: None,
            inferred: false,
        });

        let module = mb.build();

        let result = ValidateNoEscapedTypeVars.apply(module, None).unwrap();
        let escaped_diags: Vec<_> = result
            .module
            .diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::Rc(RcDiagnostic::EscapedTypeVar))
            .collect();

        // At least two diagnostics: one for return_ty, one for the field.
        assert!(
            escaped_diags.len() >= 2,
            "expected at least 2 EscapedTypeVar diagnostics (return_ty + field), got {}",
            escaped_diags.len()
        );

        // Check that the return_ty diagnostic mentions the function name.
        let return_diag = escaped_diags
            .iter()
            .find(|d| d.message.contains("return_ty"))
            .expect("expected a diagnostic mentioning return_ty");
        assert!(return_diag.message.contains("leaky_return"));

        // Check that the field diagnostic mentions the struct and field.
        let field_diag = escaped_diags
            .iter()
            .find(|d| d.message.contains("bad_field"))
            .expect("expected a diagnostic mentioning bad_field");
        assert!(field_diag.message.contains("LeakyStruct"));

        assert!(!result.changed);
    }
}
