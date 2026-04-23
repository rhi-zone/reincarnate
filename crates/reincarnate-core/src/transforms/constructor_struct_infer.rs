use std::collections::{HashMap, HashSet};

use crate::error::CoreError;
use crate::ir::{FieldDef, FuncId, Function, MethodKind, Module, Op, StructDef, Type, Visibility};
use crate::pipeline::{Transform, TransformResult};

/// Infer struct definitions from constructor and instance-method `SetField` ops.
///
/// Scans constructor functions (`MethodKind::Constructor`) and instance methods
/// (`MethodKind::Instance`) for `SetField { object: self_param, field, value }` ops
/// and synthesizes or augments a `StructDef` entry in `module.structs`.  Runs before
/// `TypeInference` so that field types are available to the solver.
///
/// Rules:
/// - Functions with `method_kind == MethodKind::Constructor` or `MethodKind::Instance`
///   are scanned.  `MethodKind::Closure` is skipped (different self convention).
/// - Fields are accumulated per class name across all matching functions, then committed
///   once per class.
/// - Only `SetField` ops whose `object` is the first entry-block parameter (the `self`
///   parameter) are collected.
/// - If a `StructDef` with the derived name already exists AND was previously inferred by
///   this pass, the class is skipped.  Frontend-declared structs (not inferred) are
///   augmented with user-defined fields.
/// - The struct name is taken from `func.class` if present; otherwise the last `::` segment
///   of the function name (read from `module.func_name`).
/// - After building the `StructDef`, the first entry-block param's type and `sig.params[0]`
///   are updated to `Type::Instance(type_id)` only for `MethodKind::Constructor` (instance
///   method self params are already typed by the frontend).
pub struct ConstructorStructInfer;

/// Determine whether a type should be replaced by an inferred struct type.
fn is_unresolved(ty: &Type) -> bool {
    matches!(ty, Type::Unknown | Type::Var(_))
}

/// Merge two field types seen at different `SetField` sites.
///
/// If one side is `Unknown` and the other is concrete, prefer the concrete
/// type (unknown-unknown → unknown; concrete-concrete equal → keep; different
/// concrete types → `Union`).
fn merge_field_type(existing: Type, new_ty: Type) -> Type {
    if existing == new_ty {
        return existing;
    }
    match (&existing, &new_ty) {
        (Type::Unknown, _) => new_ty,
        (_, Type::Unknown) => existing,
        _ => {
            // Both concrete but different — produce a Union.
            match existing {
                Type::Union(mut variants) => {
                    if !variants.contains(&new_ty) {
                        variants.push(new_ty);
                    }
                    Type::Union(variants)
                }
                other => Type::Union(vec![other, new_ty]),
            }
        }
    }
}

/// Derive the struct name for a constructor function.
fn struct_name(func: &Function, func_name: &str) -> String {
    if let Some(class) = &func.class {
        return class.clone();
    }
    // Strip any `::` namespace prefix: take the last segment.
    if let Some(pos) = func_name.rfind("::") {
        func_name[pos + 2..].to_string()
    } else {
        func_name.to_string()
    }
}

impl Transform for ConstructorStructInfer {
    fn name(&self) -> &str {
        "constructor-struct-infer"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn apply(
        &self,
        mut module: Module,
        _dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        // Build a set of already-known struct names so we can decide whether to
        // create new or augment existing.
        let known_struct_names: HashSet<String> =
            module.structs.iter().map(|s| s.name.clone()).collect();

        // Collect fields per class name across all Constructor and Instance functions.
        // Value: (accumulated fields map, constructor FuncId if seen).
        // The FuncId is used for self-param type update, which is only needed for
        // Constructor methods (Instance method self params are already typed).
        let mut per_class: HashMap<String, (HashMap<String, Type>, Option<FuncId>)> =
            HashMap::new();

        for (func_id, func) in module.functions.iter() {
            let func_name = module.func_name(func_id);
            // Scan Constructor and Instance methods for SetField on self.
            // Skip Closure — closures have a different self convention.
            let is_constructor = func.method_kind == MethodKind::Constructor;
            let is_instance = func.method_kind == MethodKind::Instance;
            if !is_constructor && !is_instance {
                continue;
            }

            let name = struct_name(func, func_name);

            // For pure-script inferred structs (not in known_struct_names): skip if
            // the struct was already fully inferred by a prior pass run.
            // For frontend-declared structs (in known_struct_names): always accumulate
            // so we capture fields from all events (Constructor + Instance).
            if !known_struct_names.contains(&name) {
                let already_inferred = module
                    .find_type(&name)
                    .map(|id| module.types[id].inferred())
                    .unwrap_or(false);
                if already_inferred {
                    continue;
                }
            }

            // Get the self param (entry block param[0]).
            let entry_block = &func.blocks[func.entry];
            let Some(self_param) = entry_block.params.first() else {
                continue;
            };
            let self_value = self_param.value;

            // Walk all instructions in all blocks, collecting SetField ops on self.
            let entry = per_class
                .entry(name)
                .or_insert_with(|| (HashMap::new(), None));

            // Record Constructor FuncId for self-param type update.
            if is_constructor && entry.1.is_none() {
                entry.1 = Some(func_id);
            }

            for (_bid, block) in func.blocks.iter() {
                for &inst_id in &block.insts {
                    if let Op::SetField {
                        object,
                        field,
                        value,
                    } = &func.insts[inst_id].op
                    {
                        if *object != self_value {
                            continue;
                        }
                        let val_ty = func.value_types[*value].clone();
                        entry
                            .0
                            .entry(field.clone())
                            .and_modify(|existing| {
                                *existing = merge_field_type(existing.clone(), val_ty.clone());
                            })
                            .or_insert(val_ty);
                    }
                }
            }
        }

        // Build the final list of classes that actually have fields to commit.
        struct Inferred {
            name: String,
            fields: Vec<FieldDef>,
            constructor_func_id: Option<FuncId>,
        }

        let mut inferred: Vec<Inferred> = per_class
            .into_iter()
            .filter_map(|(name, (field_types, constructor_func_id))| {
                if field_types.is_empty() {
                    return None;
                }
                let mut fields: Vec<FieldDef> = field_types
                    .into_iter()
                    .map(|(fname, ty)| FieldDef {
                        name: fname,
                        ty,
                        default: None,
                    })
                    .collect();
                // Stable ordering: sort fields alphabetically for determinism.
                fields.sort_by(|a, b| a.name.cmp(&b.name));
                Some(Inferred {
                    name,
                    fields,
                    constructor_func_id,
                })
            })
            .collect();
        // Stable ordering across classes.
        inferred.sort_by(|a, b| a.name.cmp(&b.name));

        let changed = !inferred.is_empty();

        for inf in inferred {
            let type_id = module.intern_type(&inf.name);

            if known_struct_names.contains(&inf.name) {
                // Struct was declared by the frontend (e.g. a GML object with
                // built-in OBJT properties).  We must NOT modify module.structs
                // here: the backend uses struct fields to emit TypeScript class
                // properties, and adding create-event fields there would cause
                // TS2564 (no initializer) and TS2416 (conflict with inherited).
                //
                // Instead, only update module.types so TypeInference can resolve
                // field types for getField calls — module.types fields are *not*
                // used for class property emission.
                let existing_fields: Vec<FieldDef> = module
                    .structs
                    .iter()
                    .find(|s| s.name == inf.name)
                    .map(|s| s.fields.clone())
                    .unwrap_or_default();
                let mut merged = existing_fields;
                for new_field in &inf.fields {
                    if let Some(ef) = merged.iter_mut().find(|f| f.name == new_field.name) {
                        ef.ty = merge_field_type(ef.ty.clone(), new_field.ty.clone());
                    } else {
                        merged.push(new_field.clone());
                    }
                }
                merged.sort_by(|a, b| a.name.cmp(&b.name));
                *module.types[type_id].fields_mut() = merged;
                module.types[type_id].set_inferred(true);
            } else {
                // Create new struct.
                module.structs.push(StructDef {
                    name: inf.name.clone(),
                    namespace: Vec::new(),
                    fields: inf.fields.clone(),
                    visibility: Visibility::Public,
                });
                *module.types[type_id].fields_mut() = inf.fields;
                module.types[type_id].set_inferred(true);
            }

            // Update the self param type in value_types and sig.params[0] — only for
            // Constructor methods.  Instance method self params are already typed by
            // the frontend.
            if let Some(constructor_func_id) = inf.constructor_func_id {
                let func = &mut module.functions[constructor_func_id];
                let entry_block_param_value = func.blocks[func.entry].params[0].value;

                let current_ty = func.value_types[entry_block_param_value].clone();
                if is_unresolved(&current_ty) {
                    func.value_types[entry_block_param_value] = Type::Instance(type_id);
                    func.blocks[func.entry].params[0].ty = Type::Instance(type_id);
                }

                if let Some(param0_ty) = func.sig.params.first_mut() {
                    if is_unresolved(param0_ty) {
                        *param0_ty = Type::Instance(type_id);
                    }
                }
            }
        }

        if !changed {
            return Ok(TransformResult {
                module,
                changed: false,
                changed_funcs: HashSet::new(),
            });
        }

        Ok(TransformResult {
            module,
            changed,
            changed_funcs: HashSet::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
    use crate::ir::ty::FunctionSig;
    use crate::ir::{FuncId, Type, Visibility};
    use crate::pipeline::Transform;

    fn make_constructor_with_fields(fields: &[(&str, Type)]) -> (Module, FuncId) {
        let sig = FunctionSig {
            params: vec![Type::Unknown], // self param
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("MyClass::create", sig, Visibility::Public);
        fb.set_class(vec![], "MyClass".to_string(), MethodKind::Constructor);

        let self_val = fb.param(0);
        for (field_name, field_ty) in fields {
            let val = match field_ty {
                Type::Float(64) => fb.const_float(0.0),
                Type::Bool => fb.const_bool(false),
                Type::String => fb.const_string(""),
                _ => fb.const_int(0, 64),
            };
            fb.set_field(self_val, *field_name, val);
        }
        fb.ret(None);

        let mut mb = ModuleBuilder::new("test");
        let func_id = mb.add_function(fb.build());
        (mb.build(), func_id)
    }

    #[test]
    fn infers_struct_from_constructor() {
        let (module, _func_id) =
            make_constructor_with_fields(&[("x", Type::Float(64)), ("y", Type::Float(64))]);
        let result = ConstructorStructInfer.apply(module, None).unwrap();
        assert!(result.changed);
        let structs = &result.module.structs;
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "MyClass");
        // Fields should be sorted alphabetically.
        assert_eq!(structs[0].fields[0].name, "x");
        assert_eq!(structs[0].fields[1].name, "y");
    }

    #[test]
    fn updates_self_param_type() {
        let (module, func_id) = make_constructor_with_fields(&[("hp", Type::Float(64))]);
        let result = ConstructorStructInfer.apply(module, None).unwrap();
        let module = result.module;
        let func = &module.functions[func_id];
        let type_id = module
            .find_type("MyClass")
            .expect("MyClass should be interned");
        assert_eq!(func.sig.params[0], Type::Instance(type_id));
        let entry_param_ty = &func.blocks[func.entry].params[0].ty;
        assert_eq!(*entry_param_ty, Type::Instance(type_id));
    }

    #[test]
    fn augments_existing_non_inferred_struct() {
        let (mut module, _func_id) = make_constructor_with_fields(&[("x", Type::Float(64))]);
        // Push a pre-existing struct (frontend-declared, not via intern_type) with no fields.
        module.structs.push(StructDef {
            name: "MyClass".to_string(),
            namespace: Vec::new(),
            fields: vec![],
            visibility: Visibility::Public,
        });
        let result = ConstructorStructInfer.apply(module, None).unwrap();
        // Should still be only 1 struct entry (not duplicated).
        assert_eq!(result.module.structs.len(), 1);
        // The pass ran and augmented, so changed should be true.
        assert!(result.changed);
        // module.structs is NOT updated for pre-existing (frontend-declared) structs to
        // avoid causing the backend to emit unwanted class property declarations.
        // Only module.types is updated for type-inference-only field resolution.
        assert_eq!(result.module.structs[0].fields.len(), 0);
        // The field "x" should be in module.types for inference.
        let type_id = result
            .module
            .find_type("MyClass")
            .expect("MyClass should be interned");
        assert_eq!(result.module.types[type_id].fields().len(), 1);
        assert_eq!(result.module.types[type_id].fields()[0].name, "x");
    }

    #[test]
    fn no_setfield_no_struct() {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("Empty::create", sig, Visibility::Public);
        fb.set_class(vec![], "Empty".to_string(), MethodKind::Constructor);
        fb.ret(None);

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();

        let result = ConstructorStructInfer.apply(module, None).unwrap();
        assert!(!result.changed);
        assert!(result.module.structs.is_empty());
    }

    #[test]
    fn skips_non_self_setfield() {
        // SetField on a non-self value should not be collected.
        let sig = FunctionSig {
            params: vec![Type::Unknown, Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("Obj::create", sig, Visibility::Public);
        fb.set_class(vec![], "Obj".to_string(), MethodKind::Constructor);

        let _self_val = fb.param(0);
        let other = fb.param(1); // not self
        let val = fb.const_int(42, 64);
        fb.set_field(other, "x", val);
        fb.ret(None);

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();

        let result = ConstructorStructInfer.apply(module, None).unwrap();
        // No fields collected from non-self SetField.
        assert!(!result.changed);
        assert!(result.module.structs.is_empty());
    }

    #[test]
    fn infers_struct_from_named_constructor() {
        // Constructor method named "Enemy::create" — should be treated as init fn.
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("Enemy::create", sig, Visibility::Public);
        fb.set_class(vec![], "Enemy".to_string(), MethodKind::Constructor);

        let self_val = fb.param(0);
        let val = fb.const_float(100.0);
        fb.set_field(self_val, "hp", val);
        fb.ret(None);

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();

        let result = ConstructorStructInfer.apply(module, None).unwrap();
        assert!(result.changed);
        let structs = &result.module.structs;
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "Enemy");
        assert_eq!(structs[0].fields[0].name, "hp");
    }
}
