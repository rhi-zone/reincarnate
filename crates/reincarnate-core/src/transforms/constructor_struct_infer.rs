use std::collections::{HashMap, HashSet};

use crate::error::CoreError;
use crate::ir::{FieldDef, FuncId, Function, MethodKind, Module, Op, StructDef, Type, Visibility};
use crate::pipeline::{Transform, TransformResult};

/// Infer struct definitions from constructor `SetField` ops.
///
/// Scans constructor functions (`MethodKind::Constructor`) for
/// `SetField { object: self_param, field, value }` ops and synthesizes a
/// `StructDef` entry in `module.structs`.  This makes struct field types
/// available to `HasField` constraint resolution and type inference before
/// those passes run.
///
/// Rules:
/// - Only functions with `method_kind == MethodKind::Constructor` are scanned.
/// - Only `SetField` ops whose `object` is the first entry-block parameter
///   (the `self` parameter) are collected.
/// - If a `StructDef` with the derived name already exists in `module.structs`,
///   the function is skipped.
/// - The struct name is taken from `func.class` if present; otherwise the last
///   `::` segment of `func.name`.
/// - After building the `StructDef`, the first entry-block param's type and
///   `sig.params[0]` are updated to `Type::Struct(name)` when they were
///   `Unknown` or `Var(_)`.
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
fn struct_name(func: &Function) -> String {
    if let Some(class) = &func.class {
        return class.clone();
    }
    // Strip any `::` namespace prefix: take the last segment.
    let name = &func.name;
    if let Some(pos) = name.rfind("::") {
        name[pos + 2..].to_string()
    } else {
        name.clone()
    }
}

impl Transform for ConstructorStructInfer {
    fn name(&self) -> &str {
        "constructor-struct-infer"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        // Build a set of already-known struct names so we don't overwrite them.
        let known_struct_names: HashSet<String> =
            module.structs.iter().map(|s| s.name.clone()).collect();

        // Collect (struct_name, fields_map) from each constructor.
        // We collect first and mutate afterwards to avoid borrow conflicts.
        struct Inferred {
            name: String,
            fields: Vec<FieldDef>,
            func_id: FuncId,
        }

        let mut inferred: Vec<Inferred> = Vec::new();

        for (func_id, func) in module.functions.iter() {
            if func.method_kind != MethodKind::Constructor {
                continue;
            }

            let name = struct_name(func);
            if known_struct_names.contains(&name) {
                continue;
            }

            // Get the self param (entry block param[0]).
            let entry_block = &func.blocks[func.entry];
            let Some(self_param) = entry_block.params.first() else {
                continue;
            };
            let self_value = self_param.value;

            // Walk all instructions in all blocks, collecting SetField ops on self.
            let mut field_types: HashMap<String, Type> = HashMap::new();
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
                        field_types
                            .entry(field.clone())
                            .and_modify(|existing| {
                                *existing = merge_field_type(existing.clone(), val_ty.clone());
                            })
                            .or_insert(val_ty);
                    }
                }
            }

            if field_types.is_empty() {
                continue;
            }

            // Stable ordering: sort fields alphabetically for determinism.
            let mut fields: Vec<FieldDef> = field_types
                .into_iter()
                .map(|(fname, ty)| FieldDef {
                    name: fname,
                    ty,
                    default: None,
                })
                .collect();
            fields.sort_by(|a, b| a.name.cmp(&b.name));

            inferred.push(Inferred {
                name,
                fields,
                func_id,
            });
        }

        let changed = !inferred.is_empty();

        for inf in inferred {
            // Add the StructDef (legacy) and intern the TypeId.
            module.structs.push(StructDef {
                name: inf.name.clone(),
                namespace: Vec::new(),
                fields: inf.fields.clone(),
                visibility: Visibility::Public,
            });
            let type_id = module.intern_type(&inf.name);
            // Update fields on the TypeDecl.
            *module.types[type_id].fields_mut() = inf.fields;
            module.types[type_id].set_inferred(true);

            // Update the self param type in value_types and sig.params[0].
            let func = &mut module.functions[inf.func_id];
            let entry_block_param_value = func.blocks[func.entry].params[0].value;

            let current_ty = func.value_types[entry_block_param_value].clone();
            if is_unresolved(&current_ty) {
                func.value_types[entry_block_param_value] = Type::Instance(type_id);
                // Also update the block param's ty field.
                func.blocks[func.entry].params[0].ty = Type::Instance(type_id);
            }

            if let Some(param0_ty) = func.sig.params.first_mut() {
                if is_unresolved(param0_ty) {
                    *param0_ty = Type::Instance(type_id);
                }
            }
        }

        // Re-check: if nothing was actually inferred, mark unchanged.
        if !changed {
            return Ok(TransformResult {
                module,
                changed: false,
            });
        }

        Ok(TransformResult { module, changed })
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
                _ => fb.const_int(0),
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
        let result = ConstructorStructInfer.apply(module).unwrap();
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
        let result = ConstructorStructInfer.apply(module).unwrap();
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
    fn skips_existing_struct() {
        let (mut module, _func_id) = make_constructor_with_fields(&[("x", Type::Float(64))]);
        module.structs.push(StructDef {
            name: "MyClass".to_string(),
            namespace: Vec::new(),
            fields: vec![],
            visibility: Visibility::Public,
        });
        let result = ConstructorStructInfer.apply(module).unwrap();
        // Should still be only 1 struct (the one we added, not a new inferred one).
        assert_eq!(result.module.structs.len(), 1);
        assert!(!result.changed);
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

        let result = ConstructorStructInfer.apply(module).unwrap();
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
        let val = fb.const_int(42);
        fb.set_field(other, "x", val);
        fb.ret(None);

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();

        let result = ConstructorStructInfer.apply(module).unwrap();
        // No fields collected from non-self SetField.
        assert!(!result.changed);
        assert!(result.module.structs.is_empty());
    }
}
