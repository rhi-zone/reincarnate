// ---------------------------------------------------------------------------
// Scaffolding — class hierarchy metadata used during emission
// ---------------------------------------------------------------------------

use std::collections::{BTreeMap, HashMap, HashSet};

use reincarnate_core::entity::PrimaryMap;
use reincarnate_core::ir::module::TypeDecl;
use reincarnate_core::ir::{ClassDef, Function, MethodKind, Module, Op, Type, TypeId};
use reincarnate_core::project::ExternalTypeDef;

use super::{qualified_class_name, ClassRegistry};

/// Pre-computed class hierarchy metadata used during emission.
pub(super) struct ClassMeta {
    pub(super) ancestor_sets: HashMap<String, HashSet<String>>,
    pub(super) method_name_sets: HashMap<String, HashSet<String>>,
    /// Methods visible in the PARENT class (not the class itself).
    /// Used to determine which methods need the `override` modifier.
    pub(super) parent_method_name_sets: HashMap<String, HashSet<String>>,
    pub(super) instance_field_sets: HashMap<String, HashSet<String>>,
    pub(super) static_method_owner_map: HashMap<String, HashMap<String, String>>,
    pub(super) static_field_owner_map: HashMap<String, HashMap<String, String>>,
    /// Static field names that are unique across the entire module → owning class short name.
    /// Used to rewrite `instance.FIELD` → `OwnerClass.FIELD` for static fields.
    pub(super) unique_static_field_map: HashMap<String, String>,
    /// Instance/Free method names that are bindable (excludes getters, setters, statics, constructors).
    pub(super) bindable_method_sets: HashMap<String, HashSet<String>>,
}

impl ClassMeta {
    pub(super) fn build(module: &Module, type_defs: &BTreeMap<String, ExternalTypeDef>) -> Self {
        let method_name_sets = build_method_name_sets(module, type_defs);
        let instance_field_sets = build_instance_field_sets(module, type_defs);
        let static_method_owner_map = build_static_method_owner_map(module);
        let static_field_owner_map = build_static_field_owner_map(module);
        let parent_method_name_sets = build_parent_member_sets(
            module,
            &method_name_sets,
            &instance_field_sets,
            &static_method_owner_map,
            &static_field_owner_map,
            type_defs,
        );
        Self {
            ancestor_sets: build_ancestor_sets(module, type_defs),
            parent_method_name_sets,
            method_name_sets,
            instance_field_sets,
            static_method_owner_map,
            static_field_owner_map,
            unique_static_field_map: build_unique_static_field_map(module),
            bindable_method_sets: build_bindable_method_sets(module, type_defs),
        }
    }
}

/// Collect all member names (fields + methods) for an external type,
/// walking its `extends` chain through type_defs.
pub(super) fn collect_external_members(
    start: &str,
    type_defs: &BTreeMap<String, ExternalTypeDef>,
) -> HashSet<String> {
    let mut members = HashSet::new();
    let mut current = Some(start);
    while let Some(name) = current {
        if let Some(def) = type_defs.get(name) {
            members.extend(def.fields.keys().cloned());
            members.extend(def.methods.keys().cloned());
            current = def.extends.as_deref();
        } else {
            break;
        }
    }
    members
}

/// Check whether an external type (or any of its ancestors) is marked `open`,
/// meaning instances may have arbitrary dynamic fields. Returns `true` if any
/// type in the inheritance chain has `open: true`.
pub(super) fn is_open_type(start: &str, type_defs: &BTreeMap<String, ExternalTypeDef>) -> bool {
    let mut current = Some(start);
    while let Some(name) = current {
        if let Some(def) = type_defs.get(name) {
            if def.open {
                return true;
            }
            current = def.extends.as_deref();
        } else {
            break;
        }
    }
    false
}

/// Validate member accesses in a function against known type definitions.
///
/// Checks `GetField` and `SetField` operations: if the object has a known
/// `Instance` or `Struct` type, verifies that the field exists in the class
/// hierarchy's instance fields, method names (getters/setters), or static fields.
pub(super) fn validate_member_accesses(
    func: &Function,
    function_class: Option<&str>,
    class_meta: &ClassMeta,
    registry: &ClassRegistry,
    short_to_qualified: &HashMap<String, String>,
    type_defs: &BTreeMap<String, ExternalTypeDef>,
    module_types: &PrimaryMap<TypeId, TypeDecl>,
) {
    for (_iid, inst) in func.insts.iter() {
        let (object, field) = match &inst.op {
            Op::GetField { object, field } => (*object, field.as_str()),
            Op::SetField { object, field, .. } => (*object, field.as_str()),
            _ => continue,
        };
        let bare = field.rsplit("::").next().unwrap_or(field);
        // Skip fields that are themselves class names (constructor references).
        if registry.lookup(field).is_some() || type_defs.contains_key(bare) {
            continue;
        }
        let ty = &func.value_types[object];
        // Resolve the type name from Instance(id).
        let type_name_storage: String;
        let type_name = match ty {
            Type::Instance(id) => {
                if let Some(named) = module_types.get(*id) {
                    if let Some(name) = named.name() {
                        type_name_storage = name.to_string();
                        type_name_storage.as_str()
                    } else {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            _ => continue,
        };
        let short = type_name.rsplit("::").next().unwrap_or(type_name);
        // Try direct qualified-name lookup first (handles duplicate short names).
        // Fall back to function's own class (disambiguates collisions like two
        // classes with the same short name), then to short-name lookup.
        let qualified = if class_meta.instance_field_sets.contains_key(type_name)
            || class_meta.method_name_sets.contains_key(type_name)
        {
            type_name
        } else if let Some(fc) = function_class {
            let fc_short = fc.rsplit("::").next().unwrap_or(fc);
            if fc_short == short {
                fc
            } else {
                match short_to_qualified.get(short) {
                    Some(qn) => qn.as_str(),
                    None => {
                        if type_defs.contains_key(short) && !is_open_type(short, type_defs) {
                            let ext_members = collect_external_members(short, type_defs);
                            if !ext_members.contains(bare) {
                                eprintln!(
                                    "warning: {short} has no member '{bare}' (in {})",
                                    func.name
                                );
                            }
                        }
                        continue;
                    }
                }
            }
        } else {
            match short_to_qualified.get(short) {
                Some(qn) => qn.as_str(),
                None => {
                    // Pure-external type — validate against type_defs.
                    if type_defs.contains_key(short) && !is_open_type(short, type_defs) {
                        let ext_members = collect_external_members(short, type_defs);
                        if !ext_members.contains(bare) {
                            eprintln!("warning: {short} has no member '{bare}' (in {})", func.name);
                        }
                    }
                    continue;
                }
            }
        };
        let has_instance_field = class_meta
            .instance_field_sets
            .get(qualified)
            .is_some_and(|f| f.contains(bare));
        let has_method = class_meta.method_name_sets.get(qualified).is_some_and(|m| {
            m.contains(bare)
                || m.contains(&format!("get_{bare}"))
                || m.contains(&format!("set_{bare}"))
        });
        let has_static_field = class_meta
            .static_field_owner_map
            .get(qualified)
            .is_some_and(|m| m.contains_key(bare));
        let has_static_method = class_meta
            .static_method_owner_map
            .get(qualified)
            .is_some_and(|m| m.contains_key(bare));
        if !has_instance_field && !has_method && !has_static_field && !has_static_method {
            // Final fallback: check external type_defs (handles local interfaces
            // that also have external type definitions with field metadata).
            if type_defs.contains_key(short) {
                if is_open_type(short, type_defs) {
                    continue;
                }
                let ext_members = collect_external_members(short, type_defs);
                if ext_members.contains(bare) {
                    continue;
                }
            }
            // Check if any ancestor in the class hierarchy is an open external type
            // (e.g. GML objects extend GMLObject which has open: true).
            if let Some(ancestors) = class_meta.ancestor_sets.get(qualified) {
                if ancestors.iter().any(|a| is_open_type(a, type_defs)) {
                    continue;
                }
            }
            eprintln!("warning: {short} has no member '{bare}' (in {})", func.name);
        }
    }
}

/// Resolve a super_class string to a ClassDef, trying qualified name first
/// (handles duplicate short names) then falling back to short-name lookup.
fn resolve_parent<'a>(
    sc: &str,
    class_by_qualified: &HashMap<String, &'a ClassDef>,
    class_by_short: &HashMap<&str, &'a ClassDef>,
) -> Option<&'a ClassDef> {
    // Try qualified first (e.g. "Items.Armors::GooArmor").
    if let Some(parent) = class_by_qualified.get(sc) {
        return Some(parent);
    }
    // Fall back to short name.
    let short = sc.rsplit("::").next().unwrap_or(sc);
    class_by_short.get(short).copied()
}

/// Build a map from qualified class name to the set of ancestor short names.
///
/// For each class, the set includes the class's own short name and the short
/// names of all superclasses reachable via `super_class` links within the module.
pub(super) fn build_ancestor_sets(
    module: &Module,
    type_defs: &BTreeMap<String, ExternalTypeDef>,
) -> HashMap<String, HashSet<String>> {
    let class_by_short: HashMap<&str, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();
    let class_by_qualified: HashMap<String, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (qualified_class_name(c), c))
        .collect();

    let mut result = HashMap::new();
    for class in &module.classes {
        let mut ancestors = HashSet::new();
        ancestors.insert(class.name.clone());
        let mut current = class;
        let mut external_start: Option<&str> = None;
        while let Some(ref sc) = current.super_class {
            let short = sc.rsplit("::").next().unwrap_or(sc);
            ancestors.insert(short.to_string());
            match resolve_parent(sc, &class_by_qualified, &class_by_short) {
                Some(parent) => current = parent,
                None => {
                    external_start = Some(short);
                    break;
                }
            }
        }
        // Continue walking through external type definitions.
        let mut ext_cur = external_start;
        while let Some(ext_name) = ext_cur {
            if let Some(def) = type_defs.get(ext_name) {
                ext_cur = def.extends.as_deref();
                if let Some(parent) = ext_cur {
                    ancestors.insert(parent.to_string());
                }
            } else {
                break;
            }
        }
        result.insert(qualified_class_name(class), ancestors);
    }
    result
}

/// Build a mapping from qualified class name → set of all method short names
/// visible through the class hierarchy (own methods + all ancestor methods).
pub(super) fn build_method_name_sets(
    module: &Module,
    type_defs: &BTreeMap<String, ExternalTypeDef>,
) -> HashMap<String, HashSet<String>> {
    let class_by_short: HashMap<&str, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();
    let class_by_qualified: HashMap<String, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (qualified_class_name(c), c))
        .collect();

    let mut result = HashMap::new();
    for class in &module.classes {
        let mut names = HashSet::new();
        let mut current = class;
        let mut external_parent: Option<&str> = None;
        loop {
            for &fid in &current.methods {
                if let Some(f) = module.functions.get(fid) {
                    if !matches!(
                        f.method_kind,
                        MethodKind::Static | MethodKind::StaticInit | MethodKind::Closure
                    ) {
                        if let Some(short) = f.name.rsplit("::").next() {
                            names.insert(short.to_string());
                            // Getters/setters use get_/set_ prefix in their function
                            // name, but are accessed as bare property names in AVM2
                            // (via GetProperty/SetProperty, not explicit calls).
                            // Add the un-prefixed property name so resolve_field can
                            // recognise them as instance members.
                            match f.method_kind {
                                MethodKind::Getter => {
                                    if let Some(prop) = short.strip_prefix("get_") {
                                        names.insert(prop.to_string());
                                    }
                                }
                                MethodKind::Setter => {
                                    if let Some(prop) = short.strip_prefix("set_") {
                                        names.insert(prop.to_string());
                                    }
                                }
                                // Other method kinds don't contribute bare property names.
                                MethodKind::Free
                                | MethodKind::Constructor
                                | MethodKind::Instance
                                | MethodKind::Static
                                | MethodKind::StaticInit
                                | MethodKind::Closure => {}
                            }
                        }
                    }
                }
            }
            // Abstract members from interface classes (no body, emitted as abstract decls).
            for m in &current.abstract_members {
                names.insert(m.name.clone());
            }
            match current.super_class {
                Some(ref sc) => {
                    let short = sc.rsplit("::").next().unwrap_or(sc);
                    match resolve_parent(sc, &class_by_qualified, &class_by_short) {
                        Some(parent) => current = parent,
                        None => {
                            external_parent = Some(short);
                            break;
                        }
                    }
                }
                None => break,
            }
        }
        // Continue walking through external type definitions.
        let mut ext_cur = external_parent;
        while let Some(ext_name) = ext_cur {
            if let Some(def) = type_defs.get(ext_name) {
                names.extend(def.methods.keys().cloned());
                ext_cur = def.extends.as_deref();
            } else {
                break;
            }
        }
        result.insert(qualified_class_name(class), names);
    }
    result
}

/// Build a mapping from qualified class name → set of member names (fields + methods)
/// visible in the PARENT class.
///
/// For a class X extending parent P:
/// - If P is an in-module class: parent members = P's method_name_sets ∪ P's instance_field_sets
///   ∪ P's static_method_owner_map keys (so static method overrides are detected).
/// - If P is an external type (e.g. GMLObject from runtime.json): walk `type_defs` from P.
/// - If X has no parent: empty set.
///
/// Used to determine which methods and fields need the `override` modifier in TypeScript output.
pub(super) fn build_parent_member_sets(
    module: &Module,
    method_name_sets: &HashMap<String, HashSet<String>>,
    instance_field_sets: &HashMap<String, HashSet<String>>,
    static_method_owner_map: &HashMap<String, HashMap<String, String>>,
    static_field_owner_map: &HashMap<String, HashMap<String, String>>,
    type_defs: &BTreeMap<String, ExternalTypeDef>,
) -> HashMap<String, HashSet<String>> {
    let class_by_short: HashMap<&str, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();
    let class_by_qualified: HashMap<String, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (qualified_class_name(c), c))
        .collect();

    let mut result = HashMap::new();
    for class in &module.classes {
        let qualified = qualified_class_name(class);
        let parent_members = if let Some(ref sc) = class.super_class {
            match resolve_parent(sc, &class_by_qualified, &class_by_short) {
                Some(parent) => {
                    // In-module parent: union of instance methods, instance fields,
                    // and static methods (for static override detection).
                    let parent_q = qualified_class_name(parent);
                    let mut names = method_name_sets.get(&parent_q).cloned().unwrap_or_default();
                    if let Some(fields) = instance_field_sets.get(&parent_q) {
                        names.extend(fields.iter().cloned());
                    }
                    if let Some(statics) = static_method_owner_map.get(&parent_q) {
                        names.extend(statics.keys().cloned());
                    }
                    if let Some(statics) = static_field_owner_map.get(&parent_q) {
                        names.extend(statics.keys().cloned());
                    }
                    names
                }
                None => {
                    // External parent: walk type_defs collecting methods and fields.
                    let short = sc.rsplit("::").next().unwrap_or(sc);
                    let mut names = HashSet::new();
                    let mut ext_cur: Option<&str> = Some(short);
                    while let Some(ext_name) = ext_cur {
                        if let Some(def) = type_defs.get(ext_name) {
                            names.extend(def.methods.keys().cloned());
                            names.extend(def.fields.keys().cloned());
                            ext_cur = def.extends.as_deref();
                        } else {
                            break;
                        }
                    }
                    names
                }
            }
        } else {
            HashSet::new()
        };
        result.insert(qualified, parent_members);
    }
    result
}

/// Build a mapping from qualified class name → set of bindable method short names.
/// Only includes Instance and Free methods — excludes Getter, Setter, Static,
/// and Constructor.  Does NOT include bare getter/setter property names.
pub(super) fn build_bindable_method_sets(
    module: &Module,
    type_defs: &BTreeMap<String, ExternalTypeDef>,
) -> HashMap<String, HashSet<String>> {
    let class_by_short: HashMap<&str, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();
    let class_by_qualified: HashMap<String, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (qualified_class_name(c), c))
        .collect();

    let mut result = HashMap::new();
    for class in &module.classes {
        let mut names = HashSet::new();
        let mut current = class;
        let mut external_parent: Option<&str> = None;
        loop {
            for &fid in &current.methods {
                if let Some(f) = module.functions.get(fid) {
                    if matches!(f.method_kind, MethodKind::Instance | MethodKind::Free) {
                        if let Some(short) = f.name.rsplit("::").next() {
                            names.insert(short.to_string());
                        }
                    }
                }
            }
            match current.super_class {
                Some(ref sc) => {
                    let short = sc.rsplit("::").next().unwrap_or(sc);
                    match resolve_parent(sc, &class_by_qualified, &class_by_short) {
                        Some(parent) => current = parent,
                        None => {
                            external_parent = Some(short);
                            break;
                        }
                    }
                }
                None => break,
            }
        }
        // Continue walking through external type definitions.
        let mut ext_cur = external_parent;
        while let Some(ext_name) = ext_cur {
            if let Some(def) = type_defs.get(ext_name) {
                names.extend(def.methods.keys().cloned());
                ext_cur = def.extends.as_deref();
            } else {
                break;
            }
        }
        result.insert(qualified_class_name(class), names);
    }
    result
}

/// Build a mapping from qualified class name → map of static method short names
/// to the owning class short name, across the full ancestor chain.  Most-derived
/// class wins when multiple ancestors define the same static method.
pub(super) fn build_static_method_owner_map(
    module: &Module,
) -> HashMap<String, HashMap<String, String>> {
    let class_by_short: HashMap<&str, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();
    let class_by_qualified: HashMap<String, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (qualified_class_name(c), c))
        .collect();

    let mut result = HashMap::new();
    for class in &module.classes {
        let mut owners: HashMap<String, String> = HashMap::new();
        let mut current = class;
        loop {
            for &fid in &current.methods {
                if let Some(f) = module.functions.get(fid) {
                    if f.method_kind == MethodKind::Static {
                        if let Some(short) = f.name.rsplit("::").next() {
                            owners
                                .entry(short.to_string())
                                .or_insert_with(|| current.name.clone());
                        }
                    }
                }
            }
            match current.super_class {
                Some(ref sc) => match resolve_parent(sc, &class_by_qualified, &class_by_short) {
                    Some(parent) => current = parent,
                    None => break,
                },
                None => break,
            }
        }
        result.insert(qualified_class_name(class), owners);
    }
    result
}

/// Build a mapping from qualified class name → map of static field short name →
/// owning class short name, walking the ancestor chain. This mirrors
/// `build_static_method_owner_map` but for static fields (both `readonly` with
/// values and mutable ones assigned in cinit).
pub(super) fn build_static_field_owner_map(
    module: &Module,
) -> HashMap<String, HashMap<String, String>> {
    let class_by_short: HashMap<&str, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();
    let class_by_qualified: HashMap<String, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (qualified_class_name(c), c))
        .collect();

    let mut result = HashMap::new();
    for class in &module.classes {
        let mut owners: HashMap<String, String> = HashMap::new();
        let mut current = class;
        loop {
            for f in &current.static_fields {
                owners
                    .entry(f.name.clone())
                    .or_insert_with(|| current.name.clone());
            }
            match current.super_class {
                Some(ref sc) => match resolve_parent(sc, &class_by_qualified, &class_by_short) {
                    Some(parent) => current = parent,
                    None => break,
                },
                None => break,
            }
        }
        result.insert(qualified_class_name(class), owners);
    }
    result
}

/// Build a map from static field short name → owning class short name, restricted
/// to names that are **unique** across the entire module (appear as a static field
/// in exactly one class, own or inherited).  Used by the Flash rewriter to rewrite
/// `someInstance.UNIQUE_STATIC_FIELD` → `OwnerClass.UNIQUE_STATIC_FIELD`.
pub(super) fn build_unique_static_field_map(module: &Module) -> HashMap<String, String> {
    // Count how many distinct owning classes each static field name maps to.
    // A name is "unique" if all classes that expose it (own or inherited) agree
    // on the same owner.
    let class_by_short: HashMap<&str, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();
    let class_by_qualified: HashMap<String, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (qualified_class_name(c), c))
        .collect();

    // field_name → set of owners seen across all classes
    let mut owners_by_field: HashMap<String, HashSet<String>> = HashMap::new();
    for class in &module.classes {
        let mut current = class;
        loop {
            for f in &current.static_fields {
                owners_by_field
                    .entry(f.name.clone())
                    .or_default()
                    .insert(current.name.clone());
            }
            match current.super_class {
                Some(ref sc) => match resolve_parent(sc, &class_by_qualified, &class_by_short) {
                    Some(parent) => current = parent,
                    None => break,
                },
                None => break,
            }
        }
    }
    // Keep only fields with a single owner.
    owners_by_field
        .into_iter()
        .filter_map(|(name, owners)| {
            if owners.len() == 1 {
                let owner = owners.into_iter().next().unwrap();
                Some((name, owner))
            } else {
                None
            }
        })
        .collect()
}

/// Build a mapping from qualified class name → set of all instance field short
/// names visible through the class hierarchy (own fields + all ancestor fields).
pub(super) fn build_instance_field_sets(
    module: &Module,
    type_defs: &BTreeMap<String, ExternalTypeDef>,
) -> HashMap<String, HashSet<String>> {
    let class_by_short: HashMap<&str, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();
    let class_by_qualified: HashMap<String, &ClassDef> = module
        .classes
        .iter()
        .map(|c| (qualified_class_name(c), c))
        .collect();

    let mut result = HashMap::new();
    for class in &module.classes {
        let mut fields = HashSet::new();
        let mut current = class;
        let mut external_parent: Option<&str> = None;
        loop {
            let struct_def = &module.structs[current.struct_index];
            for field in &struct_def.fields {
                fields.insert(field.name.clone());
            }
            match current.super_class {
                Some(ref sc) => {
                    let short = sc.rsplit("::").next().unwrap_or(sc);
                    match resolve_parent(sc, &class_by_qualified, &class_by_short) {
                        Some(parent) => current = parent,
                        None => {
                            external_parent = Some(short);
                            break;
                        }
                    }
                }
                None => break,
            }
        }
        // Continue walking through external type definitions.
        let mut ext_cur = external_parent;
        while let Some(ext_name) = ext_cur {
            if let Some(def) = type_defs.get(ext_name) {
                fields.extend(def.fields.keys().cloned());
                ext_cur = def.extends.as_deref();
            } else {
                break;
            }
        }
        result.insert(qualified_class_name(class), fields);
    }
    result
}
