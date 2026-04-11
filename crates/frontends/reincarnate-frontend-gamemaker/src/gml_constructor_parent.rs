//! GML constructor parent-type assignment pass.
//!
//! After `ConstructorStructInfer` runs (in the core pipeline), inferred
//! constructor types have `parent: None`.  In GML, every constructor-based
//! struct implicitly inherits from `GMLObject`.  This pass sets the `parent`
//! field of each `TypeDecl::Object { inferred: true, parent: None }` to the
//! `TypeId` of `GMLObject`, so the TypeScript backend can emit
//! `interface Foo extends GMLObject { … }` instead of a bare `interface Foo`.

use std::collections::HashSet;

use reincarnate_core::error::CoreError;
use reincarnate_core::ir::func::FuncId;
use reincarnate_core::ir::module::TypeDecl;
use reincarnate_core::ir::Module;
use reincarnate_core::pipeline::{PureIrPass, Transform, TransformResult};

/// Frontend pass: sets `parent = GMLObject` on all inferred constructor types.
pub struct GmlConstructorParent;

impl Transform for GmlConstructorParent {
    fn name(&self) -> &str {
        "gml-constructor-parent"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn apply(
        &self,
        mut module: Module,
        _dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        // Find the TypeId for GMLObject.  It must already exist in the module
        // because `ensure_gml_object_struct` is called during extraction.
        let Some(&gml_object_id) = module.type_names.get("GMLObject") else {
            // GMLObject is not registered — nothing to do.
            return Ok(TransformResult {
                module,
                changed: false,
                changed_funcs: HashSet::new(),
            });
        };

        // Collect TypeIds to mutate (can't borrow types immutably and mutably at once).
        let type_ids: Vec<_> = module.types.keys().collect();

        let mut changed = false;
        for id in type_ids {
            // Check if this TypeDecl needs a parent set.
            let needs_parent = match &module.types[id] {
                TypeDecl::Object {
                    inferred: true,
                    parent: None,
                    name,
                    ..
                } => name.as_deref() != Some("GMLObject"),
                _ => false,
            };
            if needs_parent {
                if let TypeDecl::Object { ref mut parent, .. } = module.types[id] {
                    *parent = Some(gml_object_id);
                    changed = true;
                }
            }
        }

        Ok(TransformResult {
            module,
            changed,
            changed_funcs: HashSet::new(),
        })
    }
}

impl PureIrPass for GmlConstructorParent {}
