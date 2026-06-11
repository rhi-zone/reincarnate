//! Redundant inherited field pruning pass.
//!
//! `ConstructorStructInfer` (CSI) records a field on a child type even when an
//! ancestor already declares that field.  When the child's inferred type is
//! `Type::Value` or `Type::InferVar`, CSI simply lacked ancestor context —
//! the type is NOT genuinely unknown, it is known on the ancestor.  Dropping
//! the child's redundant record is correct modeling, not suppression.
//!
//! This pass is **frontend-injected** (like `IntToBoolPromotion`) because the
//! moment at which parent linkage is complete is engine-specific:
//! - GML: `GmlConstructorParent` sets parent at the end of the frontend pipeline;
//!   this pass runs immediately after it.
//! - Flash: parent is set at construction (`add_class`); any tail placement after
//!   CSI/constraint-solve is correct.
//!
//! The pass is engine-neutral: it reads only `module.types[].parent` and
//! `.fields`.  No reference to GMLObject, Flash classes, or any other
//! engine concept appears here.

use std::collections::{HashMap, HashSet};

use crate::error::CoreError;
use crate::ir::func::FuncId;
use crate::ir::module::TypeDecl;
use crate::ir::ty::{Type, TypeId};
use crate::ir::Module;
use crate::pipeline::{PureIrPass, Transform, TransformResult};

/// Frontend pass: drops child fields that are already declared by an ancestor
/// when the child's type is `Unknown`/`InferVar` (abstention) or structurally
/// equal to the nearest ancestor's type (redundant re-declaration).
///
/// Keeps genuine narrowing (concrete and differs from nearest ancestor's type)
/// and brand-new fields (no ancestor declares them).
pub struct RedundantInheritedFieldPrune;

/// Returns true when `ty` is an unresolved/abstaining type.
fn is_unresolved(ty: &Type) -> bool {
    matches!(ty, Type::Value | Type::InferVar(_))
}

impl Transform for RedundantInheritedFieldPrune {
    fn name(&self) -> &str {
        "redundant-inherited-field"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn apply(
        &self,
        mut module: Module,
        _dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        // Phase A: build nearest_ancestor_type: TypeId → (field_name → Type).
        //
        // For each Object type with a parent, walk the parent chain starting from
        // the parent (skip the child itself), collecting field name → type with
        // NEAREST-ANCESTOR-WINS (entry().or_insert so the nearest ancestor's type
        // is kept when a farther ancestor re-declares the same name).
        // Cycle guard: per-walk visited set.
        let type_ids: Vec<TypeId> = module.types.keys().collect();

        let mut nearest_ancestor: HashMap<TypeId, HashMap<String, Type>> = HashMap::new();

        for &child in &type_ids {
            let parent_id = match &module.types[child] {
                TypeDecl::Object { parent, .. } => *parent,
                _ => continue,
            };
            let Some(mut pid) = parent_id else { continue };

            let mut visited: HashSet<TypeId> = HashSet::new();
            let child_map = nearest_ancestor.entry(child).or_default();

            loop {
                if !visited.insert(pid) {
                    // Cycle detected — stop.
                    break;
                }
                let (fields, next_parent) = match &module.types[pid] {
                    TypeDecl::Object { fields, parent, .. } => (fields.clone(), *parent),
                    _ => break,
                };
                for f in &fields {
                    child_map
                        .entry(f.name.clone())
                        .or_insert_with(|| f.ty.clone());
                }
                match next_parent {
                    Some(next) => pid = next,
                    None => break,
                }
            }
        }

        // Phase B: for each child with a parent, retain only fields that are
        // either brand-new (no ancestor declares the name) or genuine narrowing
        // (concrete type that differs from the nearest ancestor's type for that
        // name).  Drop when an ancestor declares the same name AND the child's
        // type is Unknown/InferVar OR structurally equal to the ancestor's type.
        let mut changed = false;

        for &child in &type_ids {
            let has_parent = match &module.types[child] {
                TypeDecl::Object { parent, .. } => parent.is_some(),
                _ => false,
            };
            if !has_parent {
                continue;
            }

            let Some(ancestor_fields) = nearest_ancestor.get(&child) else {
                continue;
            };
            // Clone the ancestor map so we can mutate module.types[child] below.
            let ancestor_fields = ancestor_fields.clone();

            let original_len = match &module.types[child] {
                TypeDecl::Object { fields, .. } => fields.len(),
                _ => continue,
            };

            // fields_mut() panics on Enum — we already know this is an Object from above.
            module.types[child].fields_mut().retain(|f| {
                match ancestor_fields.get(&f.name) {
                    // Brand-new field: no ancestor declares it — keep.
                    None => true,
                    // Ancestor declares it: drop when child abstains or matches.
                    Some(ancestor_ty) => {
                        if is_unresolved(&f.ty) {
                            // Child type is Unknown/InferVar — abstention; drop.
                            false
                        } else {
                            // Keep only genuine narrowing (differs from ancestor).
                            f.ty != *ancestor_ty
                        }
                    }
                }
            });

            let new_len = match &module.types[child] {
                TypeDecl::Object { fields, .. } => fields.len(),
                _ => continue,
            };
            changed |= new_len != original_len;
        }

        Ok(TransformResult {
            module,
            changed,
            changed_funcs: HashSet::new(),
        })
    }
}

impl PureIrPass for RedundantInheritedFieldPrune {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::func::Visibility;
    use crate::ir::module::{FieldDef, StructDef, TypeDecl};
    use crate::ir::ty::Type;

    fn make_module_with_parent_child(
        parent_fields: Vec<FieldDef>,
        child_fields: Vec<FieldDef>,
    ) -> (Module, TypeId, TypeId) {
        let mut mb = ModuleBuilder::new("test");
        let parent_id = mb.add_struct(StructDef {
            name: "Parent".to_string(),
            namespace: vec![],
            fields: parent_fields,
            visibility: Visibility::Public,
        });
        let child_id = mb.add_struct(StructDef {
            name: "Child".to_string(),
            namespace: vec![],
            fields: child_fields,
            visibility: Visibility::Public,
        });
        // Wire parent link directly on the TypeDecl.
        if let TypeDecl::Object { ref mut parent, .. } = mb.module_mut().types[child_id] {
            *parent = Some(parent_id);
        }
        (mb.build(), parent_id, child_id)
    }

    /// (i) Drop a child field whose type is Unknown when the parent declares the
    /// same name.
    #[test]
    fn drops_unresolved_inherited_field() {
        let parent_fields = vec![FieldDef {
            name: "x".to_string(),
            ty: Type::Float(64),
            default: None,
        }];
        let child_fields = vec![FieldDef {
            name: "x".to_string(),
            ty: Type::Value,
            default: None,
        }];
        let (module, _pid, cid) = make_module_with_parent_child(parent_fields, child_fields);
        let result = RedundantInheritedFieldPrune
            .apply(module, None)
            .expect("pass failed");
        assert!(result.changed);
        let fields = result.module.types[cid].fields().to_vec();
        assert!(
            fields.iter().all(|f| f.name != "x"),
            "Unknown inherited field 'x' should have been pruned"
        );
    }

    /// (ii) Drop a child field whose type is structurally equal to the parent's
    /// type (redundant re-declaration).
    #[test]
    fn drops_structurally_equal_inherited_field() {
        let parent_fields = vec![FieldDef {
            name: "score".to_string(),
            ty: Type::Float(64),
            default: None,
        }];
        let child_fields = vec![FieldDef {
            name: "score".to_string(),
            ty: Type::Float(64),
            default: None,
        }];
        let (module, _pid, cid) = make_module_with_parent_child(parent_fields, child_fields);
        let result = RedundantInheritedFieldPrune
            .apply(module, None)
            .expect("pass failed");
        assert!(result.changed);
        let fields = result.module.types[cid].fields().to_vec();
        assert!(
            fields.iter().all(|f| f.name != "score"),
            "Redundant inherited field 'score' should have been pruned"
        );
    }

    /// (iii) Keep a child field that is a genuine narrowing (concrete type that
    /// differs from the parent's type).
    #[test]
    fn keeps_genuine_narrowing() {
        let parent_fields = vec![FieldDef {
            name: "hp".to_string(),
            ty: Type::Float(64),
            default: None,
        }];
        let child_fields = vec![FieldDef {
            name: "hp".to_string(),
            ty: Type::Int(32),
            default: None,
        }];
        let (module, _pid, cid) = make_module_with_parent_child(parent_fields, child_fields);
        let result = RedundantInheritedFieldPrune
            .apply(module, None)
            .expect("pass failed");
        // The field is a genuine narrowing — the module may or may not be `changed`
        // (other fields could have been pruned), but 'hp' must remain.
        let fields = result.module.types[cid].fields().to_vec();
        assert!(
            fields
                .iter()
                .any(|f| f.name == "hp" && f.ty == Type::Int(32)),
            "Genuine narrowing field 'hp: i32' should have been kept"
        );
    }

    /// (iv) Keep a brand-new field that does not exist in any ancestor.
    #[test]
    fn keeps_brand_new_field() {
        let parent_fields = vec![FieldDef {
            name: "x".to_string(),
            ty: Type::Float(64),
            default: None,
        }];
        let child_fields = vec![FieldDef {
            name: "unique_child_field".to_string(),
            ty: Type::Bool,
            default: None,
        }];
        let (module, _pid, cid) = make_module_with_parent_child(parent_fields, child_fields);
        let result = RedundantInheritedFieldPrune
            .apply(module, None)
            .expect("pass failed");
        assert!(!result.changed, "No fields should be pruned");
        let fields = result.module.types[cid].fields().to_vec();
        assert!(
            fields.iter().any(|f| f.name == "unique_child_field"),
            "Brand-new field 'unique_child_field' should have been kept"
        );
    }

    /// (v) Cycle guard: a parent chain with a cycle terminates without panicking.
    #[test]
    fn cycle_guard_terminates() {
        let mut mb = ModuleBuilder::new("cycle_test");
        let a_id = mb.add_struct(StructDef {
            name: "A".to_string(),
            namespace: vec![],
            fields: vec![FieldDef {
                name: "f".to_string(),
                ty: Type::Float(64),
                default: None,
            }],
            visibility: Visibility::Public,
        });
        let b_id = mb.add_struct(StructDef {
            name: "B".to_string(),
            namespace: vec![],
            fields: vec![FieldDef {
                name: "f".to_string(),
                ty: Type::Value,
                default: None,
            }],
            visibility: Visibility::Public,
        });
        // A.parent = B, B.parent = A — a cycle.
        if let TypeDecl::Object { ref mut parent, .. } = mb.module_mut().types[a_id] {
            *parent = Some(b_id);
        }
        if let TypeDecl::Object { ref mut parent, .. } = mb.module_mut().types[b_id] {
            *parent = Some(a_id);
        }
        let module = mb.build();
        // Must not panic or loop forever.
        let _result = RedundantInheritedFieldPrune
            .apply(module, None)
            .expect("pass should not fail on cyclic parent chain");
    }
}
