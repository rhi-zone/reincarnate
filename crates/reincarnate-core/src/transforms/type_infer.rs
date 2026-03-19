use std::collections::{HashMap, HashSet};

use crate::error::CoreError;
use crate::ir::ty::parse_type_notation;
use crate::ir::{
    BlockId, Constant, FieldDef, Function, FunctionSig, Inst, Module, Op, StructDef,
    SystemCallTypeRule, Type, ValueId, Visibility,
};
use crate::pipeline::{Transform, TransformResult};

/// Type inference transform — refines `Dynamic` types to concrete types
/// by forward dataflow analysis with fixed-point iteration.
pub struct TypeInference;

/// Module-level type context built once before per-function inference.
struct ModuleContext {
    /// Struct name → field name → field type.
    struct_fields: HashMap<String, HashMap<String, Type>>,
    /// Class name → static field name → field type.
    static_fields: HashMap<String, HashMap<String, Type>>,
    /// Global name → type.
    global_types: HashMap<String, Type>,
    /// Function name → return type.
    func_return_types: HashMap<String, Type>,
    /// (class_short_name, bare_method_name) → return type.
    method_return_types: HashMap<(String, String), Type>,
    /// class_short_name → super_class_short_name.
    class_hierarchy: HashMap<String, Option<String>>,
    /// bare_method_name → return type (only for unambiguous names across all classes).
    unique_method_types: HashMap<String, Type>,
    /// (system, method) → type rule for SystemCall result inference.
    system_call_type_rules: HashMap<(String, String), SystemCallTypeRule>,
    /// Whether the source language implicitly returns a value from every
    /// function (mirrors `Module::implicit_return_value`).
    implicit_return_value: bool,
}

impl ModuleContext {
    fn from_module(module: &Module) -> Self {
        let mut struct_fields = HashMap::new();
        for s in &module.structs {
            let fields: HashMap<String, Type> = s
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.ty.clone()))
                .collect();
            struct_fields.insert(s.name.clone(), fields);
        }

        let global_types = module
            .globals
            .iter()
            .map(|g| (g.name.clone(), g.ty.clone()))
            .collect();

        let mut func_return_types: HashMap<String, Type> = module
            .functions
            .values()
            .map(|f| (f.name.clone(), f.sig.return_ty.clone()))
            .collect();

        // Extend with external function signatures from runtime.
        for (name, sig) in &module.external_function_sigs {
            func_return_types
                .entry(name.clone())
                .or_insert_with(|| parse_type_notation(&sig.returns));
        }

        // Build method_return_types: (class, bare_name) → return type
        let mut method_return_types = HashMap::new();
        for f in module.functions.values() {
            if f.class.is_some() {
                if let Some(bare) = f.name.rsplit("::").next() {
                    if let Some(class) = &f.class {
                        method_return_types
                            .insert((class.clone(), bare.to_string()), f.sig.return_ty.clone());
                    }
                }
            }
        }

        // Build class_hierarchy and static_fields from module.classes
        let mut class_hierarchy: HashMap<String, Option<String>> = HashMap::new();
        let mut static_fields_map: HashMap<String, HashMap<String, Type>> = HashMap::new();
        for class in &module.classes {
            let super_short = class
                .super_class
                .as_ref()
                .map(|sc| sc.rsplit("::").next().unwrap_or(sc).to_string());
            class_hierarchy.insert(class.name.clone(), super_short);
            if !class.static_fields.is_empty() {
                let fields: HashMap<String, Type> = class
                    .static_fields
                    .iter()
                    .map(|f| (f.name.clone(), f.ty.clone()))
                    .collect();
                static_fields_map.insert(class.name.clone(), fields);
            }
        }

        // Extend with external type definitions from runtime.
        for (name, ext) in &module.external_type_defs {
            // class_hierarchy: insert with parent short name
            class_hierarchy
                .entry(name.clone())
                .or_insert_with(|| ext.extends.clone());
            // struct_fields: parse field types
            if !ext.fields.is_empty() {
                let fields: HashMap<String, Type> = ext
                    .fields
                    .iter()
                    .map(|(f, t)| (f.clone(), parse_type_notation(t)))
                    .collect();
                struct_fields
                    .entry(name.clone())
                    .or_default()
                    .extend(fields);
            }
            // method_return_types: parse return types
            for (method, sig) in &ext.methods {
                method_return_types
                    .entry((name.clone(), method.clone()))
                    .or_insert_with(|| parse_type_notation(&sig.returns));
            }
        }

        // Build unique_method_types: bare names that resolve to a single return type
        let mut bare_name_types: HashMap<String, Option<Type>> = HashMap::new();
        for ((_, bare), ty) in &method_return_types {
            match bare_name_types.get(bare) {
                None => {
                    bare_name_types.insert(bare.clone(), Some(ty.clone()));
                }
                Some(Some(existing)) if *existing == *ty => {}
                Some(Some(_)) => {
                    bare_name_types.insert(bare.clone(), None);
                }
                Some(None) => {}
            }
        }
        let unique_method_types = bare_name_types
            .into_iter()
            .filter_map(|(name, ty)| ty.map(|t| (name, t)))
            .collect();

        let system_call_type_rules = module
            .system_call_type_rules
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Self {
            struct_fields,
            static_fields: static_fields_map,
            global_types,
            func_return_types,
            method_return_types,
            class_hierarchy,
            unique_method_types,
            system_call_type_rules,
            implicit_return_value: module.implicit_return_value,
        }
    }

    /// Resolve a method's return type by walking the class hierarchy.
    fn resolve_method_return_type(&self, class: &str, method: &str) -> Option<Type> {
        let mut current = Some(class.to_string());
        let max_depth = self.class_hierarchy.len();
        for _ in 0..=max_depth {
            let Some(cls) = current else { break };
            if let Some(ty) = self
                .method_return_types
                .get(&(cls.clone(), method.to_string()))
            {
                return Some(ty.clone());
            }
            current = self.class_hierarchy.get(&cls).and_then(|s| s.clone());
        }
        None
    }

    /// Resolve a field's type by walking the class hierarchy upward,
    /// checking both instance and static fields.
    fn resolve_field_type(&self, class: &str, field: &str) -> Option<Type> {
        let bare = field.rsplit("::").next().unwrap_or(field);
        let mut current = Some(class.to_string());
        let max_depth = self.class_hierarchy.len();
        for _ in 0..=max_depth {
            let Some(cls) = current else { break };
            // Check instance fields.
            if let Some(fields) = self.struct_fields.get(&cls) {
                if let Some(ty) = fields.get(bare).or_else(|| fields.get(field)) {
                    return Some(ty.clone());
                }
            }
            // Check static fields.
            if let Some(fields) = self.static_fields.get(&cls) {
                if let Some(ty) = fields.get(bare).or_else(|| fields.get(field)) {
                    return Some(ty.clone());
                }
            }
            current = self.class_hierarchy.get(&cls).and_then(|s| s.clone());
        }
        None
    }
}

/// Replace `old` with `new` only when doing so refines our knowledge.
///
/// Allowed transitions:
/// - `Dynamic → anything` (Dynamic means "completely unresolved").
/// - `Unknown → concrete` (Unknown means "we know a value exists but not its type";
///   GlobalStore may later discover the concrete type, e.g. Float(64)).
///
/// A concrete type is never replaced by another concrete type (that would widen).
fn refine(old: &Type, new: &Type) -> Option<Type> {
    let old_unresolved = matches!(old, Type::Dynamic | Type::Unknown);
    let new_concrete = !matches!(new, Type::Dynamic | Type::Unknown);
    if (old_unresolved && new_concrete) || (*old == Type::Dynamic && *new == Type::Unknown) {
        Some(new.clone())
    } else {
        None
    }
}

/// Flatten a type into a list of non-Dynamic, non-Option, non-Union members,
/// tracking nullability. This ensures unions never nest.
fn flatten_into(ty: Type, nullable: &mut bool, out: &mut Vec<Type>) {
    match ty {
        Type::Dynamic => {}
        Type::Option(inner) => {
            *nullable = true;
            flatten_into(*inner, nullable, out);
        }
        Type::Union(v) => {
            for t in v {
                flatten_into(t, nullable, out);
            }
        }
        other => {
            if !out.contains(&other) {
                out.push(other);
            }
        }
    }
}

/// Merge two types into a union, deduplicating members.
/// `Dynamic` members are dropped. `Option` is unwrapped into a nullable flag
/// and its inner type is flattened into the member list. This prevents nesting
/// from iterative inference passes.
///
/// When `nullable` is true, the result is wrapped in `Type::Option` even when
/// the base is `Dynamic`. Without this, `null | Dynamic` → `Dynamic`, and a
/// later `union_type(Dynamic, Bool)` → `Bool` — the null information is
/// permanently lost through the intermediate Dynamic. Preserving it as
/// `Option(Dynamic)` (= TS `any | null` = `any`) allows a subsequent merge
/// with a concrete type to produce the correct `Option(T)`.
fn union_type(a: Type, b: Type) -> Type {
    let mut nullable = false;
    let mut types = Vec::new();
    flatten_into(a, &mut nullable, &mut types);
    flatten_into(b, &mut nullable, &mut types);

    let base = match types.len() {
        0 => Type::Dynamic,
        1 => types.into_iter().next().unwrap(),
        _ => Type::Union(types),
    };
    if nullable {
        Type::Option(Box::new(base))
    } else {
        base
    }
}

/// Build a map from alloc ValueId → stored type, by scanning all Store instructions.
/// If all stores to a given alloc write the same concrete type, that type is recorded.
/// If stores write different concrete types, a `Type::Union` is produced.
/// If any store writes `Dynamic`, the alloc is left out of the map (stays Dynamic)
/// because `Dynamic` means "any type" and must not be silently dropped by the union.
fn build_alloc_types(func: &Function) -> HashMap<ValueId, Type> {
    let mut alloc_stores: HashMap<ValueId, Option<Type>> = HashMap::new();
    // Track allocs that receive a Dynamic store — these must stay Dynamic.
    let mut has_dynamic_store: HashSet<ValueId> = HashSet::new();

    for inst in func.insts.values() {
        if let Op::Store { ptr, value } = &inst.op {
            let stored_ty = func.value_types[*value].clone();
            if stored_ty == Type::Dynamic {
                has_dynamic_store.insert(*ptr);
                continue;
            }
            let entry = alloc_stores.entry(*ptr).or_insert(None);
            match entry {
                None => *entry = Some(stored_ty),
                Some(existing) => {
                    *existing = union_type(existing.clone(), stored_ty);
                }
            }
        }
    }

    alloc_stores
        .into_iter()
        .filter_map(|(ptr, ty)| {
            // If any store wrote Dynamic, don't narrow this alloc.
            if has_dynamic_store.contains(&ptr) {
                return None;
            }
            ty.map(|t| (ptr, t))
        })
        .collect()
}

/// Collect incoming branch arguments for each (block, param_index) pair.
fn collect_branch_args(func: &Function) -> HashMap<(BlockId, usize), Vec<ValueId>> {
    let mut incoming: HashMap<(BlockId, usize), Vec<ValueId>> = HashMap::new();

    for (_, block) in func.blocks.iter() {
        let targets = branch_target_args(&block.terminator);
        for (target_block, args) in targets {
            for (i, arg) in args.iter().enumerate() {
                incoming.entry((target_block, i)).or_default().push(*arg);
            }
        }
    }

    incoming
}

/// Extract (target_block, args) pairs from a block terminator.
fn branch_target_args(term: &crate::ir::inst::Terminator) -> Vec<(BlockId, &[ValueId])> {
    use crate::ir::inst::Terminator;
    match term {
        Terminator::Br { target, args } => vec![(*target, args.as_slice())],
        Terminator::BrIf {
            then_target,
            then_args,
            else_target,
            else_args,
            ..
        } => vec![
            (*then_target, then_args.as_slice()),
            (*else_target, else_args.as_slice()),
        ],
        Terminator::Switch { cases, default, .. } => {
            let mut targets: Vec<(BlockId, &[ValueId])> =
                cases.iter().map(|(_, b, a)| (*b, a.as_slice())).collect();
            targets.push((default.0, default.1.as_slice()));
            targets
        }
        Terminator::Return(_) => vec![],
    }
}

/// Return type of ECMAScript String prototype methods.
/// Returns `None` for unknown methods (falls back to Dynamic).
fn string_method_return_type(method: &str) -> Option<Type> {
    match method {
        // Methods that return string
        "charAt" | "concat" | "normalize" | "padEnd" | "padStart" | "repeat" | "replace"
        | "replaceAll" | "slice" | "substring" | "substr" | "toLocaleLowerCase"
        | "toLocaleUpperCase" | "toLowerCase" | "toString" | "toUpperCase" | "trim" | "trimEnd"
        | "trimStart" | "valueOf" => Some(Type::String),
        // Methods that return number
        "charCodeAt" | "codePointAt" | "indexOf" | "lastIndexOf" | "localeCompare" | "search" => {
            Some(Type::Float(64))
        }
        // Methods that return boolean
        "endsWith" | "includes" | "startsWith" => Some(Type::Bool),
        // Methods that return number (property-like)
        "length" => Some(Type::Float(64)),
        _ => None,
    }
}

/// Infer the type of an instruction's result given the current value types.
fn infer_inst_type(
    inst: &Inst,
    func: &Function,
    ctx: &ModuleContext,
    alloc_types: &HashMap<ValueId, Type>,
    const_strings: &HashMap<ValueId, String>,
) -> Option<Type> {
    let result = inst.result?;
    let current = &func.value_types[result];

    let inferred = match &inst.op {
        Op::Const(c) => c.ty(),

        // Arithmetic: propagate the type of the first operand.
        // For Add this is conservative (can be string concat in JS), so we keep it as-is.
        Op::Add(a, _) => func.value_types[*a].clone(),
        // Sub/Mul/Div/Rem are always numeric. If the lhs is Dynamic but rhs is
        // concrete (e.g. `state.get("x") - 1.0`), use the rhs type so the result
        // doesn't poison downstream inference with Dynamic.
        Op::Sub(a, b) | Op::Mul(a, b) | Op::Div(a, b) | Op::Rem(a, b) => {
            let ty_a = &func.value_types[*a];
            if *ty_a == Type::Dynamic {
                func.value_types[*b].clone()
            } else {
                ty_a.clone()
            }
        }
        Op::Neg(a) => func.value_types[*a].clone(),

        // Bitwise: propagate type of first operand.
        Op::BitAnd(a, _) | Op::BitOr(a, _) | Op::BitXor(a, _) | Op::Shl(a, _) | Op::Shr(a, _) => {
            func.value_types[*a].clone()
        }
        Op::BitNot(a) => func.value_types[*a].clone(),

        // Comparison and logic always produce Bool.
        Op::Cmp(..) | Op::Not(_) | Op::TypeCheck(..) | Op::BoolAnd(..) | Op::BoolOr(..) => {
            Type::Bool
        }

        // Cast always produces the target type.
        Op::Cast(_, ty, _) => ty.clone(),

        // Load: use tracked alloc type if available.
        Op::Load(ptr) => {
            if let Some(ty) = alloc_types.get(ptr) {
                ty.clone()
            } else {
                return None;
            }
        }

        // GetField: look up struct field type, walking class hierarchy.
        Op::GetField { object, field } => {
            match &func.value_types[*object] {
                Type::Struct(name) => ctx.resolve_field_type(name, field).unwrap_or(Type::Dynamic),
                Type::Union(members) => {
                    // Resolve the field type for each union member and join.
                    // Members that can't resolve contribute Dynamic (unknown).
                    let mut result = Type::Dynamic;
                    for member in members {
                        let member_field_ty = if let Type::Struct(name) = member {
                            ctx.resolve_field_type(name, field).unwrap_or(Type::Dynamic)
                        } else {
                            Type::Dynamic
                        };
                        result = union_type(result, member_field_ty);
                    }
                    result
                }
                _ => {
                    // When the base is Dynamic but the field name is qualified
                    // (e.g. "fl.core:UIComponent::focusManagerUsers"), extract
                    // the class name and resolve the field type.  This handles
                    // Flash scope-lookup patterns where findPropStrict returns
                    // Dynamic but the field name carries the class info.
                    if let Some(class_part) = field.rsplit("::").nth(1) {
                        // class_part is e.g. "fl.core:UIComponent" — extract short name.
                        let short = class_part.rsplit([':', '.']).next().unwrap_or(class_part);
                        ctx.resolve_field_type(short, field)?
                    } else {
                        return None;
                    }
                }
            }
        }

        // GetIndex: extract element type from Array or value type from Map.
        Op::GetIndex { collection, .. } => match &func.value_types[*collection] {
            Type::Array(elem_ty) => *elem_ty.clone(),
            Type::Map(_, val_ty) => *val_ty.clone(),
            _ => return None,
        },

        // Direct call: look up return type via 3-strategy chain.
        Op::Call { func: name, args } => {
            // Strategy 1: exact qualified name lookup.
            if let Some(ty) = ctx.func_return_types.get(name) {
                ty.clone()
            }
            // Strategy 2: receiver-based — if first arg is Struct(class), walk hierarchy.
            else if let Some(first) = args.first() {
                if let Type::Struct(class) = &func.value_types[*first] {
                    let bare = name.rsplit("::").next().unwrap_or(name);
                    ctx.resolve_method_return_type(class, bare)
                        .unwrap_or_else(|| {
                            // Strategy 3: unique bare name fallback.
                            ctx.unique_method_types
                                .get(bare)
                                .cloned()
                                .unwrap_or(Type::Dynamic)
                        })
                } else {
                    // No struct receiver — try unique bare name.
                    let bare = name.rsplit("::").next().unwrap_or(name);
                    ctx.unique_method_types
                        .get(bare)
                        .cloned()
                        .unwrap_or(Type::Dynamic)
                }
            } else {
                Type::Dynamic
            }
        }

        // MethodCall: use receiver type to look up method return type.
        Op::MethodCall {
            receiver,
            method,
            args: _,
        } => {
            let bare = method.rsplit("::").next().unwrap_or(method);
            if let Type::Struct(class) = &func.value_types[*receiver] {
                ctx.resolve_method_return_type(class, bare)
                    .unwrap_or_else(|| {
                        ctx.unique_method_types
                            .get(bare)
                            .cloned()
                            .unwrap_or(Type::Dynamic)
                    })
            } else if func.value_types[*receiver] == Type::String {
                string_method_return_type(bare).unwrap_or_else(|| {
                    ctx.unique_method_types
                        .get(bare)
                        .cloned()
                        .unwrap_or(Type::Dynamic)
                })
            } else {
                ctx.unique_method_types
                    .get(bare)
                    .cloned()
                    .unwrap_or(Type::Dynamic)
            }
        }

        // Spread: propagate source type.
        Op::Spread(v) => func.value_types[*v].clone(),

        // StructInit: always Struct(name).
        Op::StructInit { name, .. } => Type::Struct(name.clone()),

        // ArrayInit: infer element type from elements.
        Op::ArrayInit(elems) => {
            let elem_ty = infer_common_type(elems.iter().map(|v| &func.value_types[*v]));
            Type::Array(Box::new(elem_ty))
        }

        // TupleInit: collect element types.
        Op::TupleInit(elems) => {
            Type::Tuple(elems.iter().map(|v| func.value_types[*v].clone()).collect())
        }

        // GlobalRef: class constructor references get ClassRef type so that
        // callee signatures use `typeof ClassName` rather than `number`.
        Op::GlobalRef(name) => {
            if ctx.class_hierarchy.contains_key(name.as_str()) {
                Type::ClassRef(name.clone())
            } else {
                ctx.global_types.get(name).cloned().unwrap_or(Type::Dynamic)
            }
        }

        // Select: infer common type of the two branches.
        Op::Select {
            on_true, on_false, ..
        } => infer_common_type(
            [&func.value_types[*on_true], &func.value_types[*on_false]].into_iter(),
        ),

        // SystemCall: infer types from frontend-provided rules.
        Op::SystemCall {
            system,
            method,
            args,
        } => {
            let key = (system.clone(), method.clone());
            match ctx.system_call_type_rules.get(&key) {
                Some(SystemCallTypeRule::ResolveClassName) => {
                    let first = args.first()?;
                    let name = const_strings.get(first)?;
                    let bare = name.rsplit("::").next().unwrap_or(name);
                    if ctx.struct_fields.contains_key(bare)
                        || ctx.class_hierarchy.contains_key(bare)
                    {
                        Type::Struct(bare.to_string())
                    } else {
                        return None;
                    }
                }
                Some(SystemCallTypeRule::ConstructFromFirstArgType) => {
                    let first = args.first()?;
                    if let Type::Struct(name) = &func.value_types[*first] {
                        Type::Struct(name.clone())
                    } else {
                        return None;
                    }
                }
                Some(
                    SystemCallTypeRule::ResolveGlobalType
                    | SystemCallTypeRule::ResolveGlobalTypeStructOnly { .. },
                ) => {
                    let first = args.first()?;
                    let name = const_strings.get(first)?;
                    ctx.global_types
                        .get(name.as_str())
                        .cloned()
                        .unwrap_or(Type::Dynamic)
                }
                // GlobalStore is a write-side rule used by build_global_types,
                // not a result-type rule — no type to infer here.
                Some(SystemCallTypeRule::GlobalStore { .. }) | None => return None,
            }
        }

        // CallIndirect and everything else: keep current type.
        _ => return None,
    };

    refine(current, &inferred).map(|_| inferred)
}

/// Find the common type among an iterator of types.
/// Returns the single type if all agree, a `Union` if they differ,
/// or `Dynamic` if any input is `Dynamic` or the iterator is empty.
fn infer_common_type<'a>(mut types: impl Iterator<Item = &'a Type>) -> Type {
    let Some(first) = types.next() else {
        return Type::Dynamic;
    };
    if *first == Type::Dynamic {
        return Type::Dynamic;
    }
    let mut result = first.clone();
    for ty in types {
        if *ty == Type::Dynamic {
            return Type::Dynamic;
        }
        if *ty != result {
            result = union_type(result, ty.clone());
        }
    }
    result
}

/// String-only method names (not shared with Array) used to detect string-typed fields.
const STRING_ONLY_METHODS: &[&str] = &[
    "toLowerCase",
    "toUpperCase",
    "startsWith",
    "endsWith",
    "substring",
    "split",
    "replace",
    "replaceAll",
    "match",
    "matchAll",
    "search",
    "trim",
    "trimStart",
    "trimEnd",
    "trimLeft",
    "trimRight",
    "charAt",
    "charCodeAt",
    "codePointAt",
    "repeat",
    "padStart",
    "padEnd",
    "normalize",
    "localeCompare",
    "toLocaleLowerCase",
    "toLocaleUpperCase",
    // SugarCube string extensions
    "toUpperFirst",
    "toProperCase",
    "link",
    "format",
];

/// Infer the type of a struct field value (`vid`) from a single use-site instruction.
///
/// Returns `None` when this instruction provides no type information about `vid`.
/// Used by Phase 3 struct use-site inference in `build_global_types`.
fn infer_field_use_type(
    vid: ValueId,
    inst: &Inst,
    array_field_set: &HashSet<&str>,
    func: &Function,
) -> Option<Type> {
    match &inst.op {
        Op::GetField { object, field } if *object == vid => {
            if array_field_set.contains(field.as_str()) {
                Some(Type::Array(Box::new(Type::Dynamic)))
            } else {
                None
            }
        }
        Op::GetIndex { collection, .. } if *collection == vid => {
            Some(Type::Array(Box::new(Type::Dynamic)))
        }
        Op::SetIndex { collection, .. } if *collection == vid => {
            Some(Type::Array(Box::new(Type::Dynamic)))
        }
        Op::MethodCall {
            receiver, method, ..
        } if *receiver == vid => {
            if array_field_set.contains(method.as_str()) {
                Some(Type::Array(Box::new(Type::Dynamic)))
            } else if STRING_ONLY_METHODS.contains(&method.as_str()) {
                Some(Type::String)
            } else {
                None
            }
        }
        Op::CallIndirect { callee, args } if *callee == vid => {
            let params = vec![Type::Dynamic; args.len()];
            Some(Type::Function(Box::new(FunctionSig {
                params,
                return_ty: Type::Dynamic,
                ..Default::default()
            })))
        }
        Op::Add(a, b) | Op::Sub(a, b) | Op::Mul(a, b) | Op::Div(a, b) | Op::Rem(a, b)
            if *a == vid || *b == vid =>
        {
            Some(Type::Float(64))
        }
        Op::Neg(v) if *v == vid => Some(Type::Float(64)),
        Op::BitAnd(a, b) | Op::BitOr(a, b) | Op::BitXor(a, b) | Op::Shl(a, b) | Op::Shr(a, b)
            if *a == vid || *b == vid =>
        {
            Some(Type::Int(64))
        }
        Op::Not(v) if *v == vid => Some(Type::Bool),
        Op::BoolAnd(a, b) | Op::BoolOr(a, b) if *a == vid || *b == vid => Some(Type::Bool),
        Op::Select { cond, .. } if *cond == vid => Some(Type::Bool),
        // `field == "literal"` — infer field type from the other operand's known type.
        Op::Cmp(_, a, b) if *a == vid || *b == vid => {
            let other = if *a == vid { *b } else { *a };
            let other_ty = &func.value_types[other];
            match other_ty {
                Type::String => Some(Type::String),
                Type::Bool => Some(Type::Bool),
                Type::Int(w) => Some(Type::Int(*w)),
                Type::Float(w) => Some(Type::Float(*w)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Cross-function scan: collect value types from all global-store `SystemCall`
/// Remove `Array(Dynamic)` or `Array(Unknown)` members from a union when the union
/// also contains at least one concrete non-array type.
///
/// Motivation: GlobalStore write-site inference sometimes adds a spurious
/// `Array(Dynamic)` to a variable's type when one passage wraps the variable in an
/// array (e.g. `<<set _x to [_x]>>`).  If the variable is genuinely a string,
/// struct, etc. at all other write sites, the `Array(Dynamic)` member is misleading
/// and causes TS2367 ("any[] and string have no overlap") and TS2339 ("property
/// does not exist on type 'any[]'") errors.
///
/// This filter is conservative: it only removes `Array(Dynamic|Unknown)` — typed
/// arrays (`Array(String)`, `Array(Int)`, etc.) are never removed.  Pure-array
/// variables (no non-array write sites) are unaffected.
fn strip_opaque_array_from_union(ty: Type) -> Type {
    let Type::Union(mut variants) = ty else {
        return ty;
    };
    // Does the union contain at least one concrete, non-array, non-opaque type?
    let has_concrete_non_array = variants.iter().any(|t| {
        !matches!(
            t,
            Type::Array(_) | Type::Dynamic | Type::Unknown | Type::Var(_)
        )
    });
    if !has_concrete_non_array {
        return Type::Union(variants);
    }
    // Strip Array(Dynamic) and Array(Unknown) — these are "opaque array" slots.
    variants.retain(
        |v| !matches!(v, Type::Array(elem) if matches!(**elem, Type::Dynamic | Type::Unknown)),
    );
    match variants.len() {
        0 => Type::Dynamic,
        1 => variants.into_iter().next().unwrap(),
        _ => Type::Union(variants),
    }
}

/// Returns true when the union variants span more than one TypeScript type family.
///
/// TypeScript collapses `Int(_) | Float(_)` to `number`; treating them as
/// different IR types does not constitute a real conflict at the TS level.
fn ts_type_family_conflict(variants: &[Type]) -> bool {
    #[derive(PartialEq, Eq, Hash, Clone, Copy)]
    enum TsFamily {
        Number,
        Boolean,
        String,
        Array,
        Other,
    }
    let family = |t: &Type| match t {
        Type::Int(_) | Type::Float(_) | Type::UInt(_) => TsFamily::Number,
        Type::Bool => TsFamily::Boolean,
        Type::String => TsFamily::String,
        Type::Array(_) => TsFamily::Array,
        _ => TsFamily::Other,
    };
    let mut seen = std::collections::HashSet::new();
    for v in variants {
        seen.insert(family(v));
    }
    seen.len() > 1
}

/// Return type of [`build_global_types`]:
/// `(type_map, inferred_structs, string_indexed_names, write_conflicts)`.
type GlobalTypesResult = (
    HashMap<String, Type>,
    Vec<StructDef>,
    HashSet<String>,
    Vec<(String, Type)>,
);

/// Scan all GlobalStore write-site
/// instructions (identified via `SystemCallTypeRule::GlobalStore` rules registered
/// by frontends).  Returns a map from global name → inferred type, plus any
/// struct definitions inferred from use-site field access patterns (Phase 3),
/// plus the set of schema keys (var names without `_SC_` prefix) that are accessed
/// with dynamic index keys (i.e. need `[key: string]: any` in their interfaces),
/// plus a list of write-site type conflicts `(var_name, raw_union_type)` for
/// variables whose concrete write-site types span different TypeScript type families.
fn build_global_types(module: &Module) -> GlobalTypesResult {
    use crate::ir::module::SystemCallTypeRule;

    // Pre-collect the GlobalStore rules so we can match by (system, method).
    let store_rules: HashMap<(&str, &str), (usize, usize)> = module
        .system_call_type_rules
        .iter()
        .filter_map(|((sys, meth), rule)| {
            if let SystemCallTypeRule::GlobalStore {
                name_arg,
                value_arg,
            } = rule
            {
                Some(((sys.as_str(), meth.as_str()), (*name_arg, *value_arg)))
            } else {
                None
            }
        })
        .collect();

    if store_rules.is_empty() {
        return (HashMap::new(), Vec::new(), HashSet::new(), Vec::new());
    }

    // Per-variable accumulator: union of all write-site types.
    //
    // Dynamic members are dropped by `union_type` (see `flatten_into`), so
    // opaque (Dynamic/Unknown) writes contribute nothing to the union.
    // Variables written only with opaque values remain `None` and stay Dynamic.
    // Variables written with conflicting concrete types produce a Union, which
    // is detected as RC0004.
    let mut global_stores: HashMap<String, Option<Type>> = HashMap::new();
    // Schema keys (var names without `_SC_` prefix) accessed via GetIndex/SetIndex.
    let mut index_accessed_vars: HashSet<String> = HashSet::new();
    for func in module.functions.values() {
        let const_strings: HashMap<ValueId, &str> = func
            .insts
            .values()
            .filter_map(|inst| {
                if let Op::Const(Constant::String(s)) = &inst.op {
                    Some((inst.result?, s.as_str()))
                } else {
                    None
                }
            })
            .collect();

        // Build a reverse map: result ValueId → instruction, for look-through below.
        let result_to_inst: HashMap<ValueId, &crate::ir::Inst> = func
            .insts
            .values()
            .filter_map(|inst| Some((inst.result?, inst)))
            .collect();

        for inst in func.insts.values() {
            if let Op::SystemCall {
                system,
                method,
                args,
            } = &inst.op
            {
                if let Some(&(name_arg, value_arg)) =
                    store_rules.get(&(system.as_str(), method.as_str()))
                {
                    if name_arg < args.len() && value_arg < args.len() {
                        if let Some(name) = const_strings.get(&args[name_arg]) {
                            let write_val = args[value_arg];
                            let value_ty = func.value_types[write_val].clone();
                            // If the write-site value is Dynamic or Unknown, attempt a
                            // single look-through: `$x += 200` produces
                            // Add(State.get("x"), 200.0) where the get result is
                            // Dynamic/Unknown on early passes; the rhs reveals the
                            // numeric type.  If look-through fails, count as an opaque
                            // write (we know a write happened but not with what type).
                            let effective_ty: Option<Type> =
                                if matches!(value_ty, Type::Dynamic | Type::Unknown) {
                                    result_to_inst.get(&write_val).and_then(|prod| {
                                        if let Op::Add(a, b) = &prod.op {
                                            if matches!(
                                                func.value_types[*a],
                                                Type::Dynamic | Type::Unknown
                                            ) {
                                                let ty_b = &func.value_types[*b];
                                                if matches!(
                                                    ty_b,
                                                    Type::Float(_) | Type::Int(_) | Type::UInt(_)
                                                ) {
                                                    Some(ty_b.clone())
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        } else {
                                            None
                                        }
                                    })
                                } else {
                                    Some(value_ty)
                                };
                            if let Some(ty) = effective_ty {
                                let entry = global_stores.entry(name.to_string()).or_insert(None);
                                match entry {
                                    None => *entry = Some(ty),
                                    Some(existing) => *existing = union_type(existing.clone(), ty),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Use-site inference: infer global variable types from how they are used.
    //
    // Phase 2 (array/function): when a ResolveGlobalType result is accessed via
    // GetField with an array method name or called indirectly, infer the variable
    // as Array(Dynamic) or Function(…→Dynamic).
    //
    // Phase 3 (struct): when a ResolveGlobalType result is accessed via GetField
    // on a non-array field, infer a named struct type `_SC_<varname>` with field
    // types derived from how each field value is subsequently used.
    //
    // Both phases handle variables initialised in user scripts (e.g. old SC1
    // `state.active.variables.x = {…}`) that are never set via passage-level
    // `<<set>>` macros — write sites are invisible, but read patterns reveal types.

    // struct_schemas accumulates field type information across all functions.
    // Outer key: variable name.  Inner key: field name.  Value: inferred type.
    let mut struct_schemas: HashMap<String, HashMap<String, Type>> = HashMap::new();
    // write_site_fields: (struct_key, field_name) pairs confirmed to exist via
    // SetField write-site inference.  These fields are included in the emitted
    // struct interface even when their type is Dynamic (unknown → emits as `any`).
    // Fields from GetField read-site inference are excluded when Dynamic because
    // we can't distinguish "the field has type any" from "we just haven't typed it".
    let mut write_site_fields: HashSet<(String, String)> = HashSet::new();

    // constructor_names: names whose struct-only resolve results are used as
    // the callee of `SugarCube.Engine.new`.  These are JS built-in constructors
    // (Date, RegExp, etc.), not story variables.  They are excluded from
    // struct_schemas at the end so that no _SC_* interface is generated for them
    // and no cast is injected for their Engine.resolve results.
    let mut constructor_names: HashSet<String> = HashSet::new();

    // struct_skip_names: explicitly listed JS globals from ResolveGlobalTypeStructOnly
    // rules.  These names must never receive struct type inference.
    let struct_skip_names: HashSet<&str> = module
        .system_call_type_rules
        .values()
        .filter_map(|rule| {
            if let SystemCallTypeRule::ResolveGlobalTypeStructOnly { skip_names } = rule {
                Some(skip_names.iter().map(|s| s.as_str()))
            } else {
                None
            }
        })
        .flatten()
        .collect();

    {
        let resolve_rules: HashSet<(&str, &str)> = module
            .system_call_type_rules
            .iter()
            .filter_map(|((sys, meth), rule)| {
                if matches!(
                    rule,
                    SystemCallTypeRule::ResolveGlobalType
                        | SystemCallTypeRule::ResolveGlobalTypeStructOnly { .. }
                ) {
                    Some((sys.as_str(), meth.as_str()))
                } else {
                    None
                }
            })
            .collect();

        // Systems/methods that participate in Phase 3 (struct inference) ONLY —
        // Phase 2 (Array/Function use-site inference) is skipped for these so
        // that JS built-in lookups (e.g. `Engine.resolve("Date")`) are not
        // incorrectly typed as function or array values.
        let struct_only_resolve_rules: HashSet<(&str, &str)> = module
            .system_call_type_rules
            .iter()
            .filter_map(|((sys, meth), rule)| {
                if matches!(rule, SystemCallTypeRule::ResolveGlobalTypeStructOnly { .. }) {
                    Some((sys.as_str(), meth.as_str()))
                } else {
                    None
                }
            })
            .collect();

        if !resolve_rules.is_empty() {
            const ARRAY_FIELDS: &[&str] = &[
                // Standard JS Array methods / properties
                "indexOf",
                "lastIndexOf",
                "length",
                "push",
                "pop",
                "splice",
                "slice",
                "includes",
                "filter",
                "map",
                "flatMap",
                "some",
                "every",
                "forEach",
                "concat",
                "join",
                "find",
                "findIndex",
                "sort",
                "reverse",
                "fill",
                "flat",
                "shift",
                "unshift",
                "reduce",
                "reduceRight",
                "keys",
                "values",
                "entries",
                "at",
                // SugarCube Array extensions
                "last",
                "first",
                "append",
                "prepend",
                "pluck",
                "pluckMany",
                "pushUnique",
                "delete",
                "deleteAt",
                "deleteWith",
                "random",
                "randomMany",
                "shuffle",
                "count",
                "countWith",
                "includesAny",
                "includesAll",
                "toShallowClone",
                "concatUnique",
                "flatten",
            ];
            let array_field_set: HashSet<&str> = ARRAY_FIELDS.iter().copied().collect();

            for func in module.functions.values() {
                let func_const_strings: HashMap<ValueId, &str> = func
                    .insts
                    .values()
                    .filter_map(|inst| {
                        if let Op::Const(Constant::String(s)) = &inst.op {
                            Some((inst.result?, s.as_str()))
                        } else {
                            None
                        }
                    })
                    .collect();

                // result_value → variable_name for all ResolveGlobalType calls.
                // struct_only_results: values from ResolveGlobalTypeStructOnly
                // calls — Phase 2 (Array/Function inference) is skipped for these.
                let mut get_results: HashMap<ValueId, &str> = HashMap::new();
                let mut struct_only_results: HashSet<ValueId> = HashSet::new();
                for inst in func.insts.values() {
                    if let Op::SystemCall {
                        system,
                        method,
                        args,
                    } = &inst.op
                    {
                        if resolve_rules.contains(&(system.as_str(), method.as_str())) {
                            if let (Some(&name), Some(result)) = (
                                args.first().and_then(|a| func_const_strings.get(a)),
                                inst.result,
                            ) {
                                get_results.insert(result, name);
                                if struct_only_resolve_rules
                                    .contains(&(system.as_str(), method.as_str()))
                                {
                                    struct_only_results.insert(result);
                                }
                            }
                        }
                    }
                }

                // Extend get_results through Copy and single Alloc/Store/Load levels.
                //
                // Before Mem2Reg runs, a State.get result `v0` is often stored into
                // an Alloc and then Loaded back before use:
                //   v0 = State.get("x")  → get_results[v0] = "x"
                //   a  = Alloc(Dynamic)
                //   Store { ptr: a, value: v0 }
                //   v1 = Load(a)         → NOT in get_results without this extension
                //   GetField(v1, "f")    → missed without v1 in get_results
                {
                    // Track which allocs hold a State.get result.
                    let mut alloc_stored: HashMap<ValueId, &str> = HashMap::new();
                    for inst in func.insts.values() {
                        if let Op::Store { ptr, value } = &inst.op {
                            if let Some(&var_name) = get_results.get(value) {
                                alloc_stored.insert(*ptr, var_name);
                            }
                        }
                    }
                    // Extend via Load-of-tracked-alloc.
                    let mut extensions: Vec<(ValueId, &str, bool)> = Vec::new();
                    for inst in func.insts.values() {
                        if let Op::Load(ptr) = &inst.op {
                            if let (Some(&var_name), Some(result)) =
                                (alloc_stored.get(ptr), inst.result)
                            {
                                // Determine if the alloc's stored source was struct-only.
                                // (We only have the alloc ptr, not the original src here;
                                // conservatively inherit if ptr is struct-only.)
                                let so = struct_only_results.contains(ptr);
                                extensions.push((result, var_name, so));
                            }
                        }
                    }
                    for (vid, var_name, is_struct_only) in extensions {
                        get_results.entry(vid).or_insert(var_name);
                        if is_struct_only {
                            struct_only_results.insert(vid);
                        }
                    }
                }

                // GetField on a ResolveGlobalType result with an array field →
                // infer the variable as Array(Dynamic) if no write-site type.
                //
                // CallIndirect directly on a ResolveGlobalType result →
                // infer the variable as Function(Dynamic params → Dynamic).
                //
                // Both are Phase 2 patterns.  Skipped for struct-only resolve
                // rules (e.g. Engine.resolve) where built-in JS globals like
                // `Date` or `Math` would otherwise be wrongly typed as Function.
                for inst in func.insts.values() {
                    match &inst.op {
                        Op::GetField { object, field } => {
                            if array_field_set.contains(field.as_str())
                                && !struct_only_results.contains(object)
                            {
                                if let Some(&var_name) = get_results.get(object) {
                                    let entry =
                                        global_stores.entry(var_name.to_string()).or_insert(None);
                                    if entry.is_none() {
                                        *entry = Some(Type::Array(Box::new(Type::Dynamic)));
                                    }
                                }
                            }
                            // Phase 3: non-array GetField — record for struct inference below.
                        }
                        // `SugarCube.Engine.new(callee, ...args)` → callee is used
                        // as a JS constructor.  If the callee came from a struct-only
                        // resolve rule, the name is a built-in constructor (Date, etc.),
                        // not a story variable — exclude it from struct_schemas.
                        Op::SystemCall {
                            system,
                            method,
                            args,
                        } if system == "SugarCube.Engine"
                            && method == "new"
                            && !args.is_empty() =>
                        {
                            if struct_only_results.contains(&args[0]) {
                                if let Some(&var_name) = get_results.get(&args[0]) {
                                    constructor_names.insert(var_name.to_string());
                                }
                            }
                        }
                        Op::CallIndirect { callee, args } => {
                            if !struct_only_results.contains(callee) {
                                if let Some(&var_name) = get_results.get(callee) {
                                    let entry =
                                        global_stores.entry(var_name.to_string()).or_insert(None);
                                    if entry.is_none() {
                                        let params = vec![Type::Dynamic; args.len()];
                                        *entry = Some(Type::Function(Box::new(FunctionSig {
                                            params,
                                            return_ty: Type::Dynamic,
                                            ..Default::default()
                                        })));
                                    }
                                }
                            }
                        }
                        // Phase 2 (index type): when a ResolveGlobalType result is
                        // used as the index in a GetIndex/SetIndex, infer its type
                        // from the collection type:
                        //   Array(_)     → Int(64)   (numeric index)
                        //   Struct(_)    → String    (string key)
                        //   Map(key, _)  → key       (map key type)
                        //
                        // Conditioned on the collection's already-known type, so this
                        // only fires when the collection has been resolved in a prior
                        // pass.  Uses union semantics — conflicting hints produce
                        // Union(String, Int) rather than dropping the inference.
                        Op::GetIndex { collection, index }
                        | Op::SetIndex {
                            collection, index, ..
                        } => {
                            if let Some(&index_var) = get_results.get(index) {
                                if !struct_only_results.contains(index) {
                                    let inferred = match &func.value_types[*collection] {
                                        Type::Array(_) => Some(Type::Int(64)),
                                        Type::Struct(_) => Some(Type::String),
                                        Type::Map(key_ty, _) => Some(*key_ty.clone()),
                                        _ => None,
                                    };
                                    if let Some(ty) = inferred {
                                        let entry = global_stores
                                            .entry(index_var.to_string())
                                            .or_insert(None);
                                        match entry {
                                            None => *entry = Some(ty),
                                            Some(existing) => {
                                                *existing = union_type(existing.clone(), ty)
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Phase 3: transitive provenance tracking.
                //
                // Build a map from each derived ValueId to its "path" from the root
                // story variable: e.g. State.get("worn") → path=[], GetField(v_worn,
                // "neck") → path=["neck"], GetField(v_neck, "collar") → path=["neck",
                // "collar"].  Copy and Load/Store chains are followed so that Alloc-
                // based temporaries don't break the trace.
                //
                // Then infer field types from use-site patterns across the whole tree.

                const MAX_PROV_DEPTH: usize = 4;

                let mut provenance: HashMap<ValueId, (String, Vec<String>)> =
                    HashMap::with_capacity(get_results.len() * 2);
                for (&vid, &var_name) in &get_results {
                    provenance.insert(vid, (var_name.to_string(), vec![]));
                }

                // Fixed-point: extend provenance one hop per iteration.
                loop {
                    let mut new_entries: Vec<(ValueId, String, Vec<String>)> = Vec::new();
                    for inst in func.insts.values() {
                        let Some(result) = inst.result else { continue };
                        if provenance.contains_key(&result) {
                            continue;
                        }
                        match &inst.op {
                            Op::GetField { object, field }
                                if !array_field_set.contains(field.as_str())
                                    && !STRING_ONLY_METHODS.contains(&field.as_str()) =>
                            {
                                if let Some((root, path)) = provenance.get(object) {
                                    if path.len() < MAX_PROV_DEPTH {
                                        let mut p = path.clone();
                                        p.push(field.clone());
                                        new_entries.push((result, root.clone(), p));
                                    }
                                }
                            }
                            Op::Load(ptr) => {
                                if let Some((root, path)) = provenance.get(ptr) {
                                    new_entries.push((result, root.clone(), path.clone()));
                                }
                            }
                            // Provenance propagates through Cast so that SetField
                            // write-site inference can find fields written on cast
                            // values (e.g. `(Engine.resolve("Foo") as _SC_Foo).bar = v`).
                            Op::Cast(src, _, _) => {
                                if let Some((root, path)) = provenance.get(src) {
                                    new_entries.push((result, root.clone(), path.clone()));
                                }
                            }
                            _ => {}
                        }
                    }
                    // Store: give the alloc ptr the same provenance as the stored value.
                    for inst in func.insts.values() {
                        if let Op::Store { ptr, value } = &inst.op {
                            if !provenance.contains_key(ptr) {
                                if let Some((root, path)) = provenance.get(value) {
                                    new_entries.push((*ptr, root.clone(), path.clone()));
                                }
                            }
                        }
                    }
                    if new_entries.is_empty() {
                        break;
                    }
                    for (vid, root, path) in new_entries {
                        provenance.entry(vid).or_insert((root, path));
                    }
                }

                // Detect dynamic key access (GetIndex/SetIndex) on struct-provisioned
                // values.  The schema key for the indexed value is recorded so the
                // backend can emit `[key: string]: any` index signatures.
                for inst in func.insts.values() {
                    let collection = match &inst.op {
                        Op::GetIndex { collection, .. } => Some(*collection),
                        Op::SetIndex { collection, .. } => Some(*collection),
                        _ => None,
                    };
                    if let Some(col) = collection {
                        if let Some((root, path)) = provenance.get(&col) {
                            let key = if path.is_empty() {
                                root.clone()
                            } else {
                                format!("{}_{}", root, path.join("_"))
                            };
                            index_accessed_vars.insert(key);
                        }
                    }
                }

                // Values that are the 'object' of a non-array, non-string-method
                // GetField have struct children.  String-method fields (replace,
                // startsWith, etc.) do not count as struct children — they indicate
                // the value is a string, not a struct.
                let has_struct_children: HashSet<ValueId> = func
                    .insts
                    .values()
                    .filter_map(|inst| {
                        if let Op::GetField { object, field } = &inst.op {
                            if !array_field_set.contains(field.as_str())
                                && !STRING_ONLY_METHODS.contains(&field.as_str())
                                && provenance.contains_key(object)
                            {
                                return Some(*object);
                            }
                        }
                        None
                    })
                    .collect();

                // Helper: (root, path) → (schema_key, field_name).
                // schema_key is the struct name without the "_SC_" prefix:
                //   path=["neck"]         → key="worn",       field="neck"
                //   path=["neck","collar"]→ key="worn_neck",  field="collar"
                let schema_key_field = |root: &str, path: &[String]| -> (String, String) {
                    let field = path.last().unwrap().clone();
                    let parent = &path[..path.len() - 1];
                    let key = if parent.is_empty() {
                        root.to_string()
                    } else {
                        format!("{}_{}", root, parent.join("_"))
                    };
                    (key, field)
                };

                // Collect non-root entries (path non-empty) for schema updates.
                let non_root: Vec<(ValueId, String, Vec<String>)> = provenance
                    .iter()
                    .filter(|(_, (_, path))| !path.is_empty())
                    .map(|(&vid, (root, path))| (vid, root.clone(), path.clone()))
                    .collect();

                // Step A: use-site inference for leaf values (no struct children).
                for inst in func.insts.values() {
                    for (vid, root, path) in non_root
                        .iter()
                        .filter(|(v, _, _)| !has_struct_children.contains(v))
                    {
                        if let Some(ty) = infer_field_use_type(*vid, inst, &array_field_set, func) {
                            let (key, field) = schema_key_field(root, path);
                            let entry = struct_schemas
                                .entry(key)
                                .or_default()
                                .entry(field)
                                .or_insert(Type::Dynamic);
                            if *entry == Type::Dynamic {
                                *entry = ty;
                            }
                        }
                    }
                }
                // Also check terminators: BrIf cond implies Bool.
                for (_, block) in func.blocks.iter() {
                    if let crate::ir::inst::Terminator::BrIf { cond, .. } = &block.terminator {
                        for (vid, root, path) in non_root
                            .iter()
                            .filter(|(v, _, _)| !has_struct_children.contains(v))
                        {
                            if *cond == *vid {
                                let (key, field) = schema_key_field(root, path);
                                let entry = struct_schemas
                                    .entry(key)
                                    .or_default()
                                    .entry(field)
                                    .or_insert(Type::Dynamic);
                                if *entry == Type::Dynamic {
                                    *entry = Type::Bool;
                                }
                            }
                        }
                    }
                }

                // Step B: parent nodes with struct children → type as nested struct
                // (only if use-site inference didn't already set a concrete type, so
                // e.g. an array field with further GetField accesses stays array-typed).
                for (_vid, root, path) in non_root
                    .iter()
                    .filter(|(v, _, _)| has_struct_children.contains(v))
                {
                    let (key, field) = schema_key_field(root, path);
                    let nested = format!("_SC_{}_{}", root, path.join("_"));
                    let entry = struct_schemas
                        .entry(key)
                        .or_default()
                        .entry(field)
                        .or_insert(Type::Dynamic);
                    if *entry == Type::Dynamic {
                        *entry = Type::Struct(nested);
                    }
                }

                // Step C: write-site inference via SetField.
                //
                // When a story variable (or one of its nested fields) is the
                // `object` of a SetField, record the written field and its value
                // type in the struct schema.  This captures fields that are
                // assigned but never read in a way that narrows the type
                // (e.g. `<<set _modeloptions.animation_speed = "fast">>`).
                //
                // Only record concrete (non-Dynamic) types so we don't pollute
                // the schema with fields whose stored type is still unknown.
                for inst in func.insts.values() {
                    if let Op::SetField {
                        object,
                        field,
                        value,
                    } = &inst.op
                    {
                        // The object may be the root (e.g. _modeloptions itself) or a
                        // nested field — check both provenance roots and non-root entries.
                        //
                        // Note: we add the field even when val_ty is Dynamic so that
                        // fields assigned a Dynamic value (e.g. `Engine.new(...)`) are
                        // still declared on the struct.  The field EXISTS — omitting it
                        // causes TS2339 "Property does not exist".  It emits as `any`.
                        // Concrete types are preferred via `or_insert` (first writer wins).
                        if let Some((root, path)) = provenance.get(object) {
                            let val_ty = func.value_types[*value].clone();
                            // Build the schema key at the current path depth.
                            let (key, schema_field) = if path.is_empty() {
                                // Root: e.g. provenance["_modeloptions"] → key="_modeloptions", field=field
                                (root.clone(), field.clone())
                            } else {
                                // Nested path: delegate to schema_key_field with path+field appended.
                                let mut full_path = path.clone();
                                full_path.push(field.clone());
                                schema_key_field(root, &full_path)
                            };
                            let entry = struct_schemas
                                .entry(key.clone())
                                .or_default()
                                .entry(schema_field.clone())
                                .or_insert(Type::Dynamic);
                            // Prefer concrete types: only update if we have something
                            // better than what's already recorded.
                            if *entry == Type::Dynamic && val_ty != Type::Dynamic {
                                *entry = val_ty;
                            }
                            // Record that this field was proven to exist (even if type
                            // remains Dynamic — it will be emitted as `any`).
                            write_site_fields.insert((key, schema_field));
                        }
                    }
                }
            }
        }
    }

    // Post-processing Step D: read-site field declaration for confirmed structs.
    //
    // When a GetField instruction accesses a field on a value whose inferred
    // type is already a known `_SC_` struct, the field EXISTS at runtime even
    // if no write-site (SetField / GlobalStore) was ever observed for it.  Add
    // it as Dynamic (`any`) so the emitted TypeScript interface declares it,
    // preventing TS2339 "Property X does not exist on type _SC_Y".
    //
    // Guard: only extend struct schemas that already have confirmed fields —
    // this prevents false-positive struct creation from bare read accesses on
    // non-struct variables.
    {
        for func in module.functions.values() {
            for inst in func.insts.values() {
                if let Op::GetField { object, field } = &inst.op {
                    if let Type::Struct(struct_name) = &func.value_types[*object] {
                        if let Some(key) = struct_name.strip_prefix("_SC_") {
                            if struct_schemas.contains_key(key)
                                && !STRING_ONLY_METHODS.contains(&field.as_str())
                            {
                                struct_schemas
                                    .entry(key.to_string())
                                    .or_default()
                                    .entry(field.clone())
                                    .or_insert(Type::Dynamic);
                                // Mark as confirmed so the Dynamic entry is included
                                // in the emitted struct interface (see Phase 3 filter).
                                write_site_fields.insert((key.to_string(), field.clone()));
                            }
                        }
                    }
                }
            }
        }
    }

    // Post-processing: upgrade Array(Dynamic) struct fields to Record<string,any>
    // when they have named struct children in the schema.
    //
    // Some story variables are initialized as `[]` in setup code but used as
    // string-keyed dictionaries (e.g. `$C.npc.Whitney.state`).  The array init
    // sets `Array(Dynamic)` for the field, which TypeScript emits as `any[]` and
    // rejects dot-access (`npc.Whitney`) with TS2339.
    //
    // Evidence that a field is dictionary-style: named (non-array-method) GetField
    // accesses on it from other passages generate entries in the nested struct
    // schema.  When `struct_schemas["key_field"]` is non-empty, the field has
    // struct children and should be typed as `Record<string, any>` (TypeScript's
    // plain-object index type that accepts both dot and bracket string access).
    // `Type::Struct("Object")` emits as `Record<string, any>` via ts_type.
    {
        let array_dyn = Type::Array(Box::new(Type::Dynamic));
        let upgrades: Vec<(String, String)> = struct_schemas
            .iter()
            .flat_map(|(key, fields)| {
                fields
                    .iter()
                    .filter(|(field, ty)| {
                        **ty == array_dyn
                            && struct_schemas
                                .get(&format!("{key}_{field}"))
                                .map(|s| !s.is_empty())
                                .unwrap_or(false)
                    })
                    .map(|(field, _)| (key.clone(), field.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (key, field) in upgrades {
            struct_schemas
                .get_mut(&key)
                .unwrap()
                .insert(field, Type::Struct("Object".to_string()));
        }
    }

    // Remove constructor names from struct_schemas — these are JS built-in
    // constructors (Date, RegExp, etc.) accessed via Engine.resolve, not story
    // variables with struct field types.
    for name in &constructor_names {
        struct_schemas.remove(name);
    }
    // Remove explicitly skip-listed names (known JS globals from runtime overloads)
    // from struct_schemas only.  We must NOT touch global_stores here: a skip-listed
    // name like "random" may legitimately have a Float write-site via State.set(),
    // and removing it from global_stores would break State.get("random") type inference.
    for name in &struct_skip_names {
        struct_schemas.remove(*name);
    }

    // Phase 3: build StructDef instances from the collected field schemas.
    //
    // Only create a struct when at least one field has a concrete (non-Dynamic)
    // inferred type — Dynamic fields are excluded to avoid emitting `any` in
    // TypeScript interfaces (Law 4).  The variable type is set to
    // `Type::Struct("_SC_<name>")` only when no write-site type was found.
    let mut inferred_structs: Vec<StructDef> = Vec::new();
    for (var_name, field_types) in &struct_schemas {
        let mut fields: Vec<FieldDef> = field_types
            .iter()
            .filter(|(name, ty)| {
                // Include field if it has a concrete type, OR if it was proven to
                // exist by a SetField write-site (field IS real; type is just unknown).
                **ty != Type::Dynamic
                    || write_site_fields.contains(&(var_name.clone(), (*name).clone()))
            })
            .map(|(name, ty)| FieldDef {
                name: name.clone(),
                ty: ty.clone(),
                default: None,
            })
            .collect();
        if fields.is_empty() {
            continue;
        }
        fields.sort_by(|a, b| a.name.cmp(&b.name));
        let struct_name = format!("_SC_{}", var_name);
        inferred_structs.push(StructDef {
            name: struct_name.clone(),
            namespace: vec![],
            fields,
            visibility: Visibility::Public,
        });
        let entry = global_stores.entry(var_name.clone()).or_insert(None);
        if entry.is_none() {
            *entry = Some(Type::Struct(struct_name));
        }
    }

    // Emit empty placeholder interfaces for _SC_ struct types that are
    // referenced as field types in other structs but have no StructDef of
    // their own (because their schema was empty — no write/read-site evidence
    // for any fields).  Without this, TypeScript reports TS2304
    // "Cannot find name '_SC_xxx'" wherever the type appears.
    {
        let defined: HashSet<&str> = inferred_structs.iter().map(|s| s.name.as_str()).collect();
        let mut to_add: HashSet<String> = HashSet::new();
        for s in &inferred_structs {
            for f in &s.fields {
                if let Type::Struct(ref name) = f.ty {
                    if name.starts_with("_SC_") && !defined.contains(name.as_str()) {
                        to_add.insert(name.clone());
                    }
                }
            }
        }
        for name in to_add {
            inferred_structs.push(StructDef {
                name,
                namespace: vec![],
                fields: vec![],
                visibility: Visibility::Public,
            });
        }
    }

    // Build the final type map, collecting genuine type conflicts.
    //
    // Dynamic members are dropped by `union_type` (see `flatten_into`), so
    // opaque write sites do not affect the inferred type.  Variables written
    // only opaquely have no entry here and remain Dynamic.
    //
    // Conflict detection: when the concrete union spans different TypeScript type
    // families (e.g. String and Float), record the conflict for RC0004.
    // Int/Float are the same family (both `number`) and do NOT constitute a conflict.
    let mut conflicts: Vec<(String, Type)> = Vec::new();
    let type_map = global_stores
        .into_iter()
        .filter_map(|(name, ty_opt)| {
            let ty = ty_opt?;
            let stripped = strip_opaque_array_from_union(ty.clone());
            // Detect genuine type conflicts across TypeScript type families:
            //  (a) strip_opaque_array changed the type → opaque array stripped
            //      alongside non-array concrete types (Array vs non-Array)
            //  (b) stripped result is still a Union spanning multiple TS families
            let is_conflict = (stripped != ty)
                || matches!(&stripped, Type::Union(ms) if ts_type_family_conflict(ms));
            if is_conflict {
                conflicts.push((name.clone(), ty));
            }
            Some((name, stripped))
        })
        .collect();
    // Convert schema keys → struct names (prefix with "_SC_").
    let string_indexed_struct_names: HashSet<String> = index_accessed_vars
        .into_iter()
        .map(|key| format!("_SC_{}", key))
        .collect();
    (
        type_map,
        inferred_structs,
        string_indexed_struct_names,
        conflicts,
    )
}

/// Run type inference on a single function within the given module context.
/// Returns true if any types were refined.
fn infer_function(func: &mut Function, ctx: &ModuleContext) -> bool {
    let max_iters = func.value_types.len().max(1);
    let mut any_changed = false;

    // Build const_strings once — string constants don't change during inference.
    let const_strings: HashMap<ValueId, String> = func
        .insts
        .values()
        .filter_map(|inst| {
            if let Op::Const(Constant::String(s)) = &inst.op {
                Some((inst.result?, s.clone()))
            } else {
                None
            }
        })
        .collect();

    for _ in 0..max_iters {
        let mut changed = false;

        // Rebuild alloc types each iteration (they depend on value_types).
        let alloc_types = build_alloc_types(func);

        // Forward pass over all instructions.
        // Collect updates first, then apply (avoids borrow conflict).
        let updates: Vec<(ValueId, Type)> = func
            .insts
            .values()
            .filter_map(|inst| {
                let result = inst.result?;
                let new_ty = infer_inst_type(inst, func, ctx, &alloc_types, &const_strings)?;
                Some((result, new_ty))
            })
            .collect();

        for (vid, ty) in updates {
            func.value_types[vid] = ty;
            changed = true;
        }

        // Block parameter refinement: widen to union of all concrete incoming types.
        // Skip Dynamic args so back-edges (which depend on the param's own type)
        // don't poison the join and prevent convergence in loops.
        //
        // We use union_type rather than refine here because block params are join
        // points — their type must accommodate all incoming branches. If the first
        // iteration sees only one concrete branch (e.g. Bool from the false arm of
        // `&&`) and types the param as Bool, a later iteration may reveal that the
        // true arm brings a different concrete type (e.g. String). union_type widens
        // Bool → String | Bool correctly, whereas refine would block the update.
        let incoming = collect_branch_args(func);
        for (block_id, block) in func.blocks.iter() {
            for (i, param) in block.params.iter().enumerate() {
                if let Some(args) = incoming.get(&(block_id, i)) {
                    let common = infer_common_type(
                        args.iter()
                            .map(|v| &func.value_types[*v])
                            .filter(|ty| **ty != Type::Dynamic),
                    );
                    if common == Type::Dynamic {
                        // All incoming were Dynamic — no useful type info yet.
                        continue;
                    }
                    let current = func.value_types[param.value].clone();
                    let unified = union_type(current.clone(), common);
                    if unified != current {
                        func.value_types[param.value] = unified;
                        changed = true;
                    }
                }
            }
        }

        if !changed {
            break;
        }
        any_changed = true;
    }

    // Refine alloc instruction types from store analysis.
    // Update whenever the store-derived type differs from the current alloc type,
    // not just when current is Dynamic. This handles the case where the translator
    // gives an alloc a concrete type (e.g. String) but stores also include null,
    // requiring the type to be widened to Option(String).
    // Skip updates that would narrow to Dynamic (store analysis returning Dynamic
    // means no info — don't override a known type with "unknown").
    let alloc_types = build_alloc_types(func);
    for block in func.blocks.values() {
        for &inst_id in &block.insts {
            let inst = &func.insts[inst_id];
            if let Op::Alloc(ref ty) = inst.op {
                if let Some(result) = inst.result {
                    if let Some(refined) = alloc_types.get(&result) {
                        if refined != ty && *refined != Type::Dynamic {
                            func.insts[inst_id].op = Op::Alloc(refined.clone());
                            any_changed = true;
                        }
                    }
                }
            }
        }
    }

    // Post-convergence widening: block params that still receive a Dynamic argument
    // must themselves be Dynamic.  The main fixpoint loop above filters out Dynamic
    // args to avoid premature widening during early iterations when most values are
    // still unresolved (Dynamic).  After convergence, any remaining Dynamic arg is
    // genuinely dynamic (e.g. a struct-constructor result, or a call whose return
    // type can't be narrowed), so the block param must accommodate it.
    {
        let incoming = collect_branch_args(func);
        for (block_id, block) in func.blocks.iter() {
            for (i, param) in block.params.iter().enumerate() {
                if let Some(args) = incoming.get(&(block_id, i)) {
                    let has_persistent_dynamic =
                        args.iter().any(|v| func.value_types[*v] == Type::Dynamic);
                    if has_persistent_dynamic && func.value_types[param.value] != Type::Dynamic {
                        func.value_types[param.value] = Type::Dynamic;
                        any_changed = true;
                    }
                }
            }
        }
    }

    // Sync BlockParam.ty fields with value_types.
    for block in func.blocks.keys().collect::<Vec<_>>() {
        let param_vals: Vec<(usize, Type)> = func.blocks[block]
            .params
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let vty = &func.value_types[p.value];
                if p.ty != *vty {
                    Some((i, vty.clone()))
                } else {
                    None
                }
            })
            .collect();
        for (i, ty) in param_vals {
            func.blocks[block].params[i].ty = ty;
            any_changed = true;
        }
    }

    any_changed
}

impl Transform for TypeInference {
    fn name(&self) -> &str {
        "type-inference"
    }

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        let ctx = ModuleContext::from_module(&module);
        let mut changed = false;
        for func in module.functions.keys().collect::<Vec<_>>() {
            changed |= infer_function(&mut module.functions[func], &ctx);
        }

        // Sync narrowed entry-block param types back to sig.params so that
        // cross-function passes (ConstraintSolve) see the inferred types.
        for func in module.functions.values_mut() {
            let entry = func.entry;
            for (i, p) in func.blocks[entry].params.iter().enumerate() {
                if i < func.sig.params.len() && func.sig.params[i] != p.ty {
                    func.sig.params[i] = p.ty.clone();
                    changed = true;
                }
            }
        }

        // Infer return types from actual Return instructions.
        for func in module.functions.values_mut() {
            if func.sig.return_ty != Type::Dynamic {
                continue;
            }
            let mut return_types: Vec<&Type> = Vec::new();
            let mut has_void_return = false;
            for (_, block) in func.blocks.iter() {
                if let crate::ir::inst::Terminator::Return(val) = &block.terminator {
                    match val {
                        Some(v) => return_types.push(&func.value_types[*v]),
                        None => has_void_return = true,
                    }
                }
            }
            let inferred = if return_types.is_empty() {
                if ctx.implicit_return_value {
                    // Source language returns a value from every function even
                    // without an explicit `return` (e.g. GML returns 0.0 by
                    // default).  Keep the Dynamic return type so callers may
                    // still use the result.  Init-guard stubs in GMS2.3+ shared
                    // blobs have only `Return(None)` but callers index the
                    // result, causing TS7053 if narrowed to void.
                    continue;
                } else {
                    // No value-bearing returns and the language does not provide
                    // an implicit return value (e.g. Flash/AS3 `void` functions).
                    // Narrow to Void so callers do not treat the result as usable.
                    Type::Void
                }
            } else {
                infer_common_type(return_types.into_iter())
            };
            if has_void_return && inferred != Type::Dynamic && inferred != Type::Void {
                // Mixed void + value returns — keep Dynamic.
                continue;
            }
            if inferred != Type::Dynamic && func.sig.return_ty != inferred {
                func.sig.return_ty = inferred;
                changed = true;
            }
        }

        // Phase 3: cross-function global type inference from write sites.
        //
        // Iterate up to MAX_GLOBAL_INFERENCE_PASSES times. Each pass may
        // improve types inferred from arrays/structs whose element types
        // were themselves inferred in the previous pass.  For example:
        //   Pass 1: setup.Naked  = Struct("Object")  (direct struct literal)
        //   Pass 1 re-run: ArrayInit([Struct("Object"), ...]) → Array(Struct)
        //   Pass 2: setup.OutfitList = Array(Struct("Object"))
        //   Pass 2 re-run: Setup.get("OutfitList") narrowed to Array(Struct)
        const MAX_GLOBAL_INFERENCE_PASSES: usize = 4;
        let mut prev_inferred: HashMap<String, Type> = HashMap::new();
        let mut prev_struct_count: usize = 0;
        let mut last_conflicts: Vec<(String, Type)> = Vec::new();
        for _ in 0..MAX_GLOBAL_INFERENCE_PASSES {
            let (inferred_globals, new_structs, new_indexed, conflicts) =
                build_global_types(&module);
            last_conflicts = conflicts;

            // Check whether this scan produced any improvements over previous.
            let any_improved = inferred_globals.iter().any(|(k, v)| {
                v != &Type::Dynamic && prev_inferred.get(k).is_none_or(|pv| pv == &Type::Dynamic)
            });
            let has_undeclared = inferred_globals
                .keys()
                .any(|k| !module.globals.iter().any(|g| &g.name == k));
            let structs_changed = new_structs.len() != prev_struct_count
                || new_structs.iter().any(|ns| {
                    module
                        .structs
                        .iter()
                        .find(|s| s.name == ns.name)
                        .is_none_or(|s| s.fields.len() != ns.fields.len())
                });

            if !any_improved
                && !has_undeclared
                && prev_inferred == inferred_globals
                && !structs_changed
            {
                break;
            }
            prev_inferred = inferred_globals.clone();
            prev_struct_count = new_structs.len();

            // Update Module::globals entries for declared globals.
            for g in &mut module.globals {
                // Update Dynamic *or* Unknown globals: Unknown means "value exists
                // but type unknown"; if GlobalStore found a concrete type, use it.
                if matches!(g.ty, Type::Dynamic | Type::Unknown) {
                    if let Some(inferred) = inferred_globals.get(&g.name) {
                        if *inferred != Type::Dynamic && *inferred != Type::Unknown {
                            g.ty = inferred.clone();
                            changed = true;
                        }
                    }
                }
            }

            // Phase 3: register inferred struct definitions.
            // Replace any existing _SC_* struct from a prior pass with the updated one.
            for new_struct in new_structs {
                module.structs.retain(|s| s.name != new_struct.name);
                module.structs.push(new_struct);
                changed = true;
            }

            // Record struct names that need `[key: string]: any` index signatures.
            module.string_indexed_structs.extend(new_indexed);

            // Re-run per-function inference so that ResolveGlobalType uses
            // the newly inferred types (including for undeclared globals like
            // SugarCube story/setup variables that have no Module::globals entry).
            let mut ctx = ModuleContext::from_module(&module);
            for (name, ty) in &inferred_globals {
                if *ty == Type::Dynamic || *ty == Type::Unknown {
                    ctx.global_types
                        .entry(name.clone())
                        .or_insert_with(|| ty.clone());
                } else {
                    // Override Unknown/Dynamic in ctx from module.globals with the
                    // concrete inferred type — or_insert_with would silently keep Unknown.
                    ctx.global_types.insert(name.clone(), ty.clone());
                }
            }
            for func in module.functions.keys().collect::<Vec<_>>() {
                changed |= infer_function(&mut module.functions[func], &ctx);
            }
        }

        // Emit RC0004 diagnostics for write-site type conflicts detected in the
        // final inference pass.  Deduplicate by variable name so repeated passes
        // don't multiply the diagnostics.
        {
            use crate::pipeline::checker::{Diagnostic, DiagnosticCode, RcDiagnostic, Severity};
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for (var_name, raw_ty) in last_conflicts {
                if seen.insert(var_name.clone()) {
                    module.diagnostics.push(Diagnostic {
                        file: module.name.clone(),
                        line: 0,
                        col: 0,
                        code: DiagnosticCode::Rc(RcDiagnostic::WriteConflict),
                        severity: Severity::Error,
                        message: format!(
                            "variable `{var_name}` has conflicting write-site types: {raw_ty:?}"
                        ),
                    });
                }
            }
        }

        Ok(TransformResult { module, changed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityRef;
    use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
    use crate::ir::ty::FunctionSig;
    use crate::ir::{ClassDef, CmpKind, FieldDef, FuncId, Global, StructDef, Visibility};

    // ---- Identity & idempotency tests ----

    /// All types already concrete → no changes.
    #[test]
    fn identity_no_change() {
        let sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0);
        let c = fb.const_int(42);
        let sum = fb.add(p, c);
        fb.ret(Some(sum));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        let result = TypeInference.apply(module).unwrap();
        assert!(!result.changed);
    }

    /// Type inference is idempotent.
    #[test]
    fn idempotent_after_transform() {
        use crate::transforms::util::test_helpers::assert_idempotent;
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let a = fb.const_int(42);
        let b = fb.const_int(10);
        let sum = fb.add(a, b);
        fb.ret(Some(sum));
        let mut func = fb.build();
        func.value_types[sum] = Type::Dynamic;
        assert_idempotent(&TypeInference, func);
    }

    /// Constants propagate: pushbyte 42 + add should infer Int(64) for the add result.
    #[test]
    fn constants_propagate_through_arithmetic() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let a = fb.const_int(42);
        let b = fb.const_int(10);
        // Manually create add with Dynamic type to simulate flash frontend output.
        let sum = fb.add(a, b);
        fb.ret(Some(sum));
        let mut func = fb.build();

        // Force the add result to Dynamic (simulating untyped frontend).
        func.value_types[sum] = Type::Dynamic;

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[sum], Type::Int(64));
    }

    /// Local variable tracking: store Int(32) to alloc, load should infer Int(32).
    #[test]
    fn local_variable_tracking() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(32),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let ptr = fb.alloc(Type::Int(32));
        let val = fb.const_int(42);
        fb.store(ptr, val);
        let loaded = fb.load(ptr, Type::Dynamic); // Frontend doesn't know the type.
        fb.ret(Some(loaded));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[loaded], Type::Int(64)); // Constant::Int is always Int(64).
    }

    /// Call return type: call to known function gets the function's return type.
    #[test]
    fn call_return_type() {
        // Create a callee function.
        let callee_sig = FunctionSig {
            params: vec![],
            return_ty: Type::String,
            ..Default::default()
        };
        let mut callee_fb = FunctionBuilder::new("get_name", callee_sig, Visibility::Public);
        let s = callee_fb.const_string("hello");
        callee_fb.ret(Some(s));
        let callee = callee_fb.build();

        // Create a caller that calls get_name with Dynamic return type.
        let caller_sig = FunctionSig {
            params: vec![],
            return_ty: Type::String,
            ..Default::default()
        };
        let mut caller_fb = FunctionBuilder::new("caller", caller_sig, Visibility::Public);
        let result = caller_fb.call("get_name", &[], Type::Dynamic);
        caller_fb.ret(Some(result));
        let caller = caller_fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(callee);
        mb.add_function(caller);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let caller_func = &module.functions[FuncId::new(1)];
        assert_eq!(caller_func.value_types[result], Type::String);
    }

    /// Struct field type: GetField on a known struct resolves field type.
    #[test]
    fn struct_field_type() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let vx = fb.const_int(10);
        let vy = fb.const_int(20);
        let obj = fb.struct_init("Point", vec![("x".into(), vx), ("y".into(), vy)]);
        let x = fb.get_field(obj, "x", Type::Dynamic);
        fb.ret(Some(x));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_struct(StructDef {
            name: "Point".into(),
            namespace: Vec::new(),
            fields: vec![
                FieldDef {
                    name: "x".into(),
                    ty: Type::Int(64),
                    default: None,
                },
                FieldDef {
                    name: "y".into(),
                    ty: Type::Int(64),
                    default: None,
                },
            ],
            visibility: Visibility::Public,
        });
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[x], Type::Int(64));
    }

    /// GetField on a Union-typed object resolves the field type for each member and joins.
    /// When all members have the same concrete field type, the result is that type.
    #[test]
    fn union_getfield_same_field_type() {
        let sig = FunctionSig {
            params: vec![Type::Union(vec![
                Type::Struct("Point".into()),
                Type::Struct("Point3D".into()),
            ])],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let obj = fb.param(0);
        let x = fb.get_field(obj, "x", Type::Dynamic);
        fb.ret(Some(x));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_struct(StructDef {
            name: "Point".into(),
            namespace: Vec::new(),
            fields: vec![
                FieldDef {
                    name: "x".into(),
                    ty: Type::Int(64),
                    default: None,
                },
                FieldDef {
                    name: "y".into(),
                    ty: Type::Int(64),
                    default: None,
                },
            ],
            visibility: Visibility::Public,
        });
        mb.add_struct(StructDef {
            name: "Point3D".into(),
            namespace: Vec::new(),
            fields: vec![
                FieldDef {
                    name: "x".into(),
                    ty: Type::Int(64),
                    default: None,
                },
                FieldDef {
                    name: "y".into(),
                    ty: Type::Int(64),
                    default: None,
                },
                FieldDef {
                    name: "z".into(),
                    ty: Type::Int(64),
                    default: None,
                },
            ],
            visibility: Visibility::Public,
        });
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        // Both Point and Point3D have `x: Int(64)` → result is Int(64), not Dynamic.
        assert_eq!(func.value_types[x], Type::Int(64));
    }

    /// GetField on a Union-typed object where members have different field types
    /// produces a Union of those types.
    #[test]
    fn union_getfield_mixed_field_types() {
        let sig = FunctionSig {
            params: vec![Type::Union(vec![
                Type::Struct("Foo".into()),
                Type::Struct("Bar".into()),
            ])],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let obj = fb.param(0);
        let val = fb.get_field(obj, "val", Type::Dynamic);
        fb.ret(Some(val));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_struct(StructDef {
            name: "Foo".into(),
            namespace: Vec::new(),
            fields: vec![FieldDef {
                name: "val".into(),
                ty: Type::Int(64),
                default: None,
            }],
            visibility: Visibility::Public,
        });
        mb.add_struct(StructDef {
            name: "Bar".into(),
            namespace: Vec::new(),
            fields: vec![FieldDef {
                name: "val".into(),
                ty: Type::String,
                default: None,
            }],
            visibility: Visibility::Public,
        });
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        // Foo.val: Int(64), Bar.val: String → Union([Int(64), String]).
        assert_eq!(
            func.value_types[val],
            Type::Union(vec![Type::Int(64), Type::String])
        );
    }

    /// Block parameter join: two branches sending Int(32) to a block param → param becomes Int(32).
    #[test]
    fn block_parameter_join_same_type() {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let cond = fb.param(0);

        // Merge block with a Dynamic param.
        let (merge, merge_vals) = fb.create_block_with_params(&[Type::Dynamic]);
        let then_block = fb.create_block();
        let else_block = fb.create_block();

        fb.br_if(cond, then_block, &[], else_block, &[]);

        fb.switch_to_block(then_block);
        let a = fb.const_int(1);
        fb.br(merge, &[a]);

        fb.switch_to_block(else_block);
        let b = fb.const_int(2);
        fb.br(merge, &[b]);

        fb.switch_to_block(merge);
        fb.ret(Some(merge_vals[0]));

        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        // Both branches send Int(64), so the merge param should be Int(64).
        assert_eq!(func.value_types[merge_vals[0]], Type::Int(64));
        // BlockParam.ty should also be synced.
        assert_eq!(func.blocks[merge].params[0].ty, Type::Int(64));
    }

    /// A block param that receives a persistently-Dynamic argument must remain
    /// Dynamic after convergence.  GML struct-constructor results (e.g.
    /// `@@NewGMLObject@@`) stay `dyn` forever; a merge param that receives one
    /// such value alongside concrete values must be widened to Dynamic.
    #[test]
    fn dynamic_input_widens_block_param_at_convergence() {
        // Function: if (cond) { let x = <Dynamic call>; merge(x) }
        //           else      { let y = 0 (i64);        merge(y) }
        // The merge param starts Dynamic, gets tentatively narrowed toward Int(64)
        // by the concrete branch, but must end as Dynamic because the other branch
        // sends a genuinely-Dynamic value.
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let cond = fb.param(0);

        let (merge, merge_vals) = fb.create_block_with_params(&[Type::Dynamic]);
        let then_block = fb.create_block();
        let else_block = fb.create_block();

        fb.br_if(cond, then_block, &[], else_block, &[]);

        fb.switch_to_block(then_block);
        // A Dynamic value: entry param typed Dynamic.  TypeInference can't narrow it.
        let dynamic_val = fb.param(0); // reuse cond (Bool) — but actually we need a Dynamic val.
                                       // Use a system call whose return type is Dynamic (TypeInference can't narrow it).
        let dyn_val = fb.system_call("Ext", "create", &[], Type::Dynamic);
        fb.br(merge, &[dyn_val]);

        fb.switch_to_block(else_block);
        let zero = fb.const_int(0); // Int(64)
        fb.br(merge, &[zero]);

        fb.switch_to_block(merge);
        fb.ret(Some(merge_vals[0]));

        // Discard unused `dynamic_val`
        let _ = dynamic_val;

        let func = fb.build();
        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        // The merge param receives (Dynamic, Int(64)) — must stay Dynamic.
        assert_eq!(
            func.value_types[merge_vals[0]],
            Type::Dynamic,
            "block param receiving a genuinely-Dynamic arg must remain Dynamic"
        );
    }

    /// Mixed types produce Union: branches sending different types → Union.
    #[test]
    fn mixed_types_produce_union() {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let cond = fb.param(0);

        let (merge, merge_vals) = fb.create_block_with_params(&[Type::Dynamic]);
        let then_block = fb.create_block();
        let else_block = fb.create_block();

        fb.br_if(cond, then_block, &[], else_block, &[]);

        fb.switch_to_block(then_block);
        let a = fb.const_int(1); // Int(64)
        fb.br(merge, &[a]);

        fb.switch_to_block(else_block);
        let b = fb.const_string("hello"); // String
        fb.br(merge, &[b]);

        fb.switch_to_block(merge);
        fb.ret(Some(merge_vals[0]));

        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        // Different types → Union.
        assert_eq!(
            func.value_types[merge_vals[0]],
            Type::Union(vec![Type::Int(64), Type::String])
        );
    }

    /// GlobalRef resolves to the global's declared type.
    #[test]
    fn global_ref_type() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let g = fb.global_ref("counter", Type::Dynamic);
        fb.ret(Some(g));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_global(Global {
            name: "counter".into(),
            ty: Type::Int(64),
            visibility: Visibility::Private,
            mutable: true,
            init: None,
        });
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[g], Type::Int(64));
    }

    /// Comparison operations always produce Bool.
    #[test]
    fn comparison_produces_bool() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let a = fb.const_int(1);
        let b = fb.const_int(2);
        let cmp = fb.cmp(CmpKind::Lt, a, b);
        fb.ret(Some(cmp));
        let mut func = fb.build();

        // Force cmp result to Dynamic.
        func.value_types[cmp] = Type::Dynamic;

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[cmp], Type::Bool);
    }

    /// Helper: build a module with a class method and a caller that invokes it
    /// via a bare name on a typed receiver.
    fn build_method_call_module(
        class_name: &str,
        method_bare: &str,
        method_return_ty: Type,
        super_class: Option<&str>,
    ) -> (Module, ValueId) {
        // The method: Creature::isNaga -> Bool
        let method_full = format!("{class_name}::{method_bare}");
        let method_sig = FunctionSig {
            params: vec![Type::Struct(class_name.to_string())],
            return_ty: method_return_ty,
            ..Default::default()
        };
        let mut method_fb = FunctionBuilder::new(&method_full, method_sig, Visibility::Public);
        let self_param = method_fb.param(0);
        method_fb.ret(Some(self_param));
        let mut method_func = method_fb.build();
        method_func.class = Some(class_name.to_string());

        // The caller calls bare "isNaga" with a Struct("Creature") receiver.
        let caller_sig = FunctionSig {
            params: vec![Type::Struct(class_name.to_string())],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut caller_fb = FunctionBuilder::new("caller", caller_sig, Visibility::Public);
        let recv = caller_fb.param(0);
        let result = caller_fb.call(method_bare, &[recv], Type::Dynamic);
        caller_fb.ret(Some(result));
        let caller_func = caller_fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_struct(StructDef {
            name: class_name.into(),
            namespace: Vec::new(),
            fields: vec![],
            visibility: Visibility::Public,
        });
        let method_id = mb.add_function(method_func);
        mb.add_function(caller_func);
        mb.add_class(ClassDef {
            name: class_name.into(),
            namespace: Vec::new(),
            struct_index: 0,
            methods: vec![method_id],
            super_class: super_class.map(|s| s.to_string()),
            visibility: Visibility::Public,
            static_fields: vec![],
            is_interface: false,
            interfaces: vec![],
            abstract_members: vec![],
            is_dynamic: false,
            zero_initialized: false,
            needs_index_signature: false,
        });
        (mb.build(), result)
    }

    /// Method call resolved via receiver type.
    #[test]
    fn method_call_resolved_via_receiver() {
        let (module, result) = build_method_call_module("Creature", "isNaga", Type::Bool, None);
        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let caller = &module.functions[FuncId::new(1)];
        assert_eq!(caller.value_types[result], Type::Bool);
    }

    /// Method call resolved via hierarchy walk (method on parent class).
    #[test]
    fn method_call_resolved_via_hierarchy() {
        // Parent class "Creature" has isNaga, child "Naga" extends it.
        let parent_method_sig = FunctionSig {
            params: vec![Type::Struct("Creature".to_string())],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut parent_fb =
            FunctionBuilder::new("Creature::isNaga", parent_method_sig, Visibility::Public);
        let self_param = parent_fb.param(0);
        parent_fb.ret(Some(self_param));
        let mut parent_func = parent_fb.build();
        parent_func.class = Some("Creature".to_string());

        // Caller has a Naga receiver, calls bare "isNaga".
        let caller_sig = FunctionSig {
            params: vec![Type::Struct("Naga".to_string())],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut caller_fb = FunctionBuilder::new("caller", caller_sig, Visibility::Public);
        let recv = caller_fb.param(0);
        let result = caller_fb.call("isNaga", &[recv], Type::Dynamic);
        caller_fb.ret(Some(result));
        let caller_func = caller_fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_struct(StructDef {
            name: "Creature".into(),
            namespace: Vec::new(),
            fields: vec![],
            visibility: Visibility::Public,
        });
        mb.add_struct(StructDef {
            name: "Naga".into(),
            namespace: Vec::new(),
            fields: vec![],
            visibility: Visibility::Public,
        });
        let parent_method_id = mb.add_function(parent_func);
        mb.add_function(caller_func);
        mb.add_class(ClassDef {
            name: "Creature".into(),
            namespace: Vec::new(),
            struct_index: 0,
            methods: vec![parent_method_id],
            super_class: None,
            visibility: Visibility::Public,
            static_fields: vec![],
            is_interface: false,
            interfaces: vec![],
            abstract_members: vec![],
            is_dynamic: false,
            zero_initialized: false,
            needs_index_signature: false,
        });
        mb.add_class(ClassDef {
            name: "Naga".into(),
            namespace: Vec::new(),
            struct_index: 1,
            methods: vec![],
            super_class: Some("Creature".to_string()),
            visibility: Visibility::Public,
            static_fields: vec![],
            is_interface: false,
            interfaces: vec![],
            abstract_members: vec![],
            is_dynamic: false,
            zero_initialized: false,
            needs_index_signature: false,
        });
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let caller = &module.functions[FuncId::new(1)];
        assert_eq!(caller.value_types[result], Type::Bool);
    }

    /// Unique bare name fallback when receiver is not a Struct.
    #[test]
    fn method_call_unique_fallback() {
        // Only one class defines "isNaga" → unique fallback works.
        let method_sig = FunctionSig {
            params: vec![Type::Dynamic],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut method_fb =
            FunctionBuilder::new("Creature::isNaga", method_sig, Visibility::Public);
        let self_param = method_fb.param(0);
        method_fb.ret(Some(self_param));
        let mut method_func = method_fb.build();
        method_func.class = Some("Creature".to_string());

        // Caller with Dynamic receiver.
        let caller_sig = FunctionSig {
            params: vec![Type::Dynamic],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut caller_fb = FunctionBuilder::new("caller", caller_sig, Visibility::Public);
        let recv = caller_fb.param(0);
        let result = caller_fb.call("isNaga", &[recv], Type::Dynamic);
        caller_fb.ret(Some(result));
        let caller_func = caller_fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_struct(StructDef {
            name: "Creature".into(),
            namespace: Vec::new(),
            fields: vec![],
            visibility: Visibility::Public,
        });
        let method_id = mb.add_function(method_func);
        mb.add_function(caller_func);
        mb.add_class(ClassDef {
            name: "Creature".into(),
            namespace: Vec::new(),
            struct_index: 0,
            methods: vec![method_id],
            super_class: None,
            visibility: Visibility::Public,
            static_fields: vec![],
            is_interface: false,
            interfaces: vec![],
            abstract_members: vec![],
            is_dynamic: false,
            zero_initialized: false,
            needs_index_signature: false,
        });
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let caller = &module.functions[FuncId::new(1)];
        assert_eq!(caller.value_types[result], Type::Bool);
    }

    /// Select with both branches Int(64) infers Int(64).
    #[test]
    fn select_same_type_inferred() {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let cond = fb.param(0);
        let a = fb.const_int(1);
        let b = fb.const_int(2);
        let mut func = fb.build();

        // Manually insert a Select with Dynamic result type.
        let select_val = func.value_types.push(Type::Dynamic);
        let select_inst = func.insts.push(Inst {
            op: Op::Select {
                cond,
                on_true: a,
                on_false: b,
            },
            result: Some(select_val),
            span: None,
        });
        let entry = BlockId::new(0);
        // Insert before the terminator.
        let term_pos = func.blocks[entry].insts.len() - 1;
        func.blocks[entry].insts.insert(term_pos, select_inst);

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[select_val], Type::Int(64));
    }

    /// Select with mixed types produces Union.
    #[test]
    fn select_mixed_types_produces_union() {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let cond = fb.param(0);
        let a = fb.const_int(1);
        let b = fb.const_string("hello");
        let mut func = fb.build();

        let select_val = func.value_types.push(Type::Dynamic);
        let select_inst = func.insts.push(Inst {
            op: Op::Select {
                cond,
                on_true: a,
                on_false: b,
            },
            result: Some(select_val),
            span: None,
        });
        let entry = BlockId::new(0);
        let term_pos = func.blocks[entry].insts.len() - 1;
        func.blocks[entry].insts.insert(term_pos, select_inst);

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(
            func.value_types[select_val],
            Type::Union(vec![Type::Int(64), Type::String])
        );
    }

    /// Ambiguous bare name stays Dynamic when multiple classes disagree on return type.
    #[test]
    fn method_call_ambiguous_stays_dynamic() {
        // Two classes define "getValue" with different return types.
        let method1_sig = FunctionSig {
            params: vec![Type::Dynamic],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut method1_fb =
            FunctionBuilder::new("ClassA::getValue", method1_sig, Visibility::Public);
        let s1 = method1_fb.param(0);
        method1_fb.ret(Some(s1));
        let mut method1 = method1_fb.build();
        method1.class = Some("ClassA".to_string());

        let method2_sig = FunctionSig {
            params: vec![Type::Dynamic],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut method2_fb =
            FunctionBuilder::new("ClassB::getValue", method2_sig, Visibility::Public);
        let s2 = method2_fb.param(0);
        method2_fb.ret(Some(s2));
        let mut method2 = method2_fb.build();
        method2.class = Some("ClassB".to_string());

        // Caller with Dynamic receiver.
        let caller_sig = FunctionSig {
            params: vec![Type::Dynamic],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut caller_fb = FunctionBuilder::new("caller", caller_sig, Visibility::Public);
        let recv = caller_fb.param(0);
        let result = caller_fb.call("getValue", &[recv], Type::Dynamic);
        caller_fb.ret(Some(result));
        let caller_func = caller_fb.build();

        let mut mb = ModuleBuilder::new("test");
        let m1_id = mb.add_function(method1);
        let m2_id = mb.add_function(method2);
        mb.add_function(caller_func);
        mb.add_class(ClassDef {
            name: "ClassA".into(),
            namespace: Vec::new(),
            struct_index: 0,
            methods: vec![m1_id],
            super_class: None,
            visibility: Visibility::Public,
            static_fields: vec![],
            is_interface: false,
            interfaces: vec![],
            abstract_members: vec![],
            is_dynamic: false,
            zero_initialized: false,
            needs_index_signature: false,
        });
        mb.add_class(ClassDef {
            name: "ClassB".into(),
            namespace: Vec::new(),
            struct_index: 0,
            methods: vec![m2_id],
            super_class: None,
            visibility: Visibility::Public,
            static_fields: vec![],
            is_interface: false,
            interfaces: vec![],
            abstract_members: vec![],
            is_dynamic: false,
            zero_initialized: false,
            needs_index_signature: false,
        });
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let caller = &module.functions[FuncId::new(2)];
        // Ambiguous — stays Dynamic.
        assert_eq!(caller.value_types[result], Type::Dynamic);
    }

    /// Alloc(Dynamic) refined to Alloc(Int(64)) when all stores agree.
    #[test]
    fn alloc_type_refined_from_stores() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let ptr = fb.alloc(Type::Dynamic);
        let a = fb.const_int(1);
        fb.store(ptr, a);
        let b = fb.const_int(2);
        fb.store(ptr, b);
        let loaded = fb.load(ptr, Type::Dynamic);
        fb.ret(Some(loaded));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        // The alloc op should now be Alloc(Int(64)).
        let alloc_inst = func
            .insts
            .values()
            .find(|i| matches!(&i.op, Op::Alloc(_)))
            .unwrap();
        match &alloc_inst.op {
            Op::Alloc(ty) => assert_eq!(*ty, Type::Int(64)),
            other => panic!("expected Alloc, got {:?}", other),
        }
    }

    /// Alloc(Dynamic) becomes Alloc(Union([Int(64), String])) when stores disagree.
    #[test]
    fn alloc_type_union_from_mixed_stores() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let ptr = fb.alloc(Type::Dynamic);
        let a = fb.const_int(1);
        fb.store(ptr, a);
        let b = fb.const_string("hello");
        fb.store(ptr, b);
        let loaded = fb.load(ptr, Type::Dynamic);
        fb.ret(Some(loaded));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        // Mixed stores — alloc becomes Union.
        let alloc_inst = func
            .insts
            .values()
            .find(|i| matches!(&i.op, Op::Alloc(_)))
            .unwrap();
        match &alloc_inst.op {
            Op::Alloc(ty) => assert_eq!(*ty, Type::Union(vec![Type::Int(64), Type::String])),
            other => panic!("expected Alloc, got {:?}", other),
        }
    }

    /// Null sentinel + concrete type → Option(ConcreteType).
    #[test]
    fn alloc_type_null_sentinel_absorbed() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let ptr = fb.alloc(Type::Dynamic);
        let a = fb.const_int(1);
        fb.store(ptr, a);
        let b = fb.const_null();
        fb.store(ptr, b);
        let loaded = fb.load(ptr, Type::Dynamic);
        fb.ret(Some(loaded));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        let alloc_inst = func
            .insts
            .values()
            .find(|i| matches!(&i.op, Op::Alloc(_)))
            .unwrap();
        match &alloc_inst.op {
            Op::Alloc(ty) => assert_eq!(*ty, Type::Option(Box::new(Type::Int(64)))),
            other => panic!("expected Alloc, got {:?}", other),
        }
    }

    /// Null sentinel + Dynamic → Option(Dynamic), preserving nullability.
    /// Even though `Option(Dynamic)` = `any | null` = `any` in TypeScript,
    /// keeping the nullable flag in the IR is critical: if this alloc later
    /// receives a concrete-typed store, union_type(Option(Dynamic), Bool)
    /// produces Option(Bool) rather than losing the null and producing Bool.
    #[test]
    fn alloc_type_null_sentinel_with_dynamic_stays_dynamic() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let ptr = fb.alloc(Type::Dynamic);
        let b = fb.const_null();
        fb.store(ptr, b);
        let loaded = fb.load(ptr, Type::Dynamic);
        fb.ret(Some(loaded));
        let mut func = fb.build();

        // Simulate an unresolved store by manually inserting a Store with Dynamic value.
        let dyn_val = func.value_types.push(Type::Dynamic);
        let store_inst = func.insts.push(Inst {
            op: Op::Store {
                ptr,
                value: dyn_val,
            },
            result: None,
            span: None,
        });
        let entry = BlockId::new(0);
        let term_pos = func.blocks[entry].insts.len() - 1;
        func.blocks[entry].insts.insert(term_pos, store_inst);

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        let alloc_inst = func
            .insts
            .values()
            .find(|i| matches!(&i.op, Op::Alloc(_)))
            .unwrap();
        match &alloc_inst.op {
            // Dynamic allocs that receive at least one Dynamic store stay Dynamic
            // (the null sentinel is subsumed by Dynamic).
            Op::Alloc(ty) => assert_eq!(*ty, Type::Dynamic),
            other => panic!("expected Alloc, got {:?}", other),
        }
    }

    /// Alloc pre-typed as String + null store → widened to Option(String).
    /// This covers the case where the translator sets an explicit concrete type
    /// on an alloc, but runtime code assigns null (e.g. SugarCube `_temp` vars).
    #[test]
    fn alloc_type_widened_to_option_when_null_stored() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let ptr = fb.alloc(Type::String); // pre-typed as String
        let null_val = fb.const_null();
        fb.store(ptr, null_val);
        let str_val = fb.const_string("hello".to_string());
        fb.store(ptr, str_val);
        let loaded = fb.load(ptr, Type::Dynamic);
        fb.ret(Some(loaded));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        let alloc_inst = func
            .insts
            .values()
            .find(|i| matches!(&i.op, Op::Alloc(_)))
            .unwrap();
        match &alloc_inst.op {
            Op::Alloc(ty) => assert_eq!(*ty, Type::Option(Box::new(Type::String))),
            other => panic!("expected Alloc, got {:?}", other),
        }
    }

    // ---- Edge case tests ----

    /// Void function — no types to refine.
    #[test]
    fn void_function_noop() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        fb.ret(None);

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        let result = TypeInference.apply(module).unwrap();
        assert!(!result.changed);
    }

    /// Dynamic + Int(64) in Add — forward-only inference keeps result Dynamic.
    /// (Backward constraint flow is ConstraintSolve's job.)
    #[test]
    fn dynamic_operand_stays_dynamic_in_add() {
        let sig = FunctionSig {
            params: vec![Type::Dynamic],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0);
        let c = fb.const_int(1);
        let sum = fb.add(p, c);
        fb.ret(Some(sum));
        let mut func = fb.build();
        func.value_types[sum] = Type::Dynamic;

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();
        let result = TypeInference.apply(module).unwrap();
        let func = &result.module.functions[FuncId::new(0)];
        // TypeInference only does forward flow: Dynamic + Int → Dynamic.
        assert_eq!(func.value_types[sum], Type::Dynamic);
    }

    // ---- Adversarial tests ----

    /// Circular block params: loop where block param feeds back to itself.
    #[test]
    fn circular_block_params() {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let cond = fb.param(0);
        let init = fb.const_int(0);

        let (header, header_params) = fb.create_block_with_params(&[Type::Dynamic]);
        let body = fb.create_block();
        let exit = fb.create_block();

        fb.br(header, &[init]);

        fb.switch_to_block(header);
        fb.br_if(cond, body, &[], exit, &[]);

        fb.switch_to_block(body);
        let one = fb.const_int(1);
        let sum = fb.add(header_params[0], one);
        fb.br(header, &[sum]);

        fb.switch_to_block(exit);
        fb.ret(Some(header_params[0]));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        // Should not infinite loop or panic.
        let result = TypeInference.apply(module).unwrap();
        let func = &result.module.functions[FuncId::new(0)];
        // The param should settle to Int(64) from the concrete branches.
        assert_eq!(func.value_types[header_params[0]], Type::Int(64));
    }

    /// Deeply nested field chain with Dynamic root.
    #[test]
    fn deeply_nested_field_chain() {
        let sig = FunctionSig {
            params: vec![Type::Dynamic],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0);
        let a = fb.get_field(p, "a", Type::Dynamic);
        let b = fb.get_field(a, "b", Type::Dynamic);
        let c = fb.get_field(b, "c", Type::Dynamic);
        fb.ret(Some(c));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        // Should not panic.
        let result = TypeInference.apply(module).unwrap();
        let func = &result.module.functions[FuncId::new(0)];
        // No struct info → all stay Dynamic.
        assert_eq!(func.value_types[c], Type::Dynamic);
    }

    /// Cross-function global type inference: a set in one function types a get in another.
    #[test]
    fn global_type_inferred_from_write_site() {
        // Writer function: global.set("score", 42)
        let writer_sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut writer_fb = FunctionBuilder::new("writer", writer_sig, Visibility::Public);
        let name = writer_fb.const_string("score");
        let val = writer_fb.const_int(42);
        writer_fb.system_call("GameMaker.Global", "set", &[name, val], Type::Void);
        writer_fb.ret(None);
        let writer = writer_fb.build();

        // Reader function: global.get("score") — should resolve to Int(64)
        let reader_sig = FunctionSig {
            params: vec![],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut reader_fb = FunctionBuilder::new("reader", reader_sig, Visibility::Public);
        let name2 = reader_fb.const_string("score");
        let result = reader_fb.system_call("GameMaker.Global", "get", &[name2], Type::Dynamic);
        reader_fb.ret(Some(result));
        let reader = reader_fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_global(Global {
            name: "score".into(),
            ty: Type::Dynamic,
            visibility: Visibility::Public,
            mutable: true,
            init: None,
        });
        mb.add_function(writer);
        mb.add_function(reader);
        let mut module = mb.build();
        module.system_call_type_rules.insert(
            ("GameMaker.Global".into(), "get".into()),
            SystemCallTypeRule::ResolveGlobalType,
        );
        module.system_call_type_rules.insert(
            ("GameMaker.Global".into(), "set".into()),
            SystemCallTypeRule::GlobalStore {
                name_arg: 0,
                value_arg: 1,
            },
        );

        let transform = TypeInference;
        let module = transform.apply(module).unwrap().module;

        // Global type should be inferred from the write site.
        let score_global = module.globals.iter().find(|g| g.name == "score").unwrap();
        assert_eq!(score_global.ty, Type::Int(64));

        // Reader's get result should now be Int(64).
        let reader_func = &module.functions[FuncId::new(1)];
        assert_eq!(reader_func.value_types[result], Type::Int(64));
    }

    /// MethodCall on a String receiver infers return type from the method name.
    #[test]
    fn string_method_call_return_type() {
        let sig = FunctionSig {
            params: vec![Type::String],
            return_ty: Type::Dynamic,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let s = fb.param(0);
        let pattern = fb.const_string("a");
        let replacement = fb.const_string("b");
        // call_method with Dynamic return type — simulating untyped frontend output.
        let replaced = fb.call_method(s, "replace", &[pattern, replacement], Type::Dynamic);
        let idx = fb.call_method(s, "indexOf", &[pattern], Type::Dynamic);
        let starts = fb.call_method(s, "startsWith", &[pattern], Type::Dynamic);
        fb.ret(Some(replaced));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        let module = TypeInference.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(
            func.value_types[replaced],
            Type::String,
            "replace returns String"
        );
        assert_eq!(
            func.value_types[idx],
            Type::Float(64),
            "indexOf returns Float(64)"
        );
        assert_eq!(
            func.value_types[starts],
            Type::Bool,
            "startsWith returns Bool"
        );
    }

    /// String method inference + RedCastElim eliminates redundant coerce_s.
    #[test]
    fn string_replace_coerce_eliminated() {
        let sig = FunctionSig {
            params: vec![Type::String],
            return_ty: Type::String,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let s = fb.param(0);
        let pattern = fb.const_string("a");
        let replacement = fb.const_string("b");
        // Simulate: v1 = s.replace(a, b) → Dynamic, then coerce to String.
        let replaced = fb.call_method(s, "replace", &[pattern, replacement], Type::Dynamic);
        let coerced = fb.coerce(replaced, Type::String);
        fb.ret(Some(coerced));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();

        // After type inference, replaced should be String, making the coerce redundant.
        let module = TypeInference.apply(module).unwrap().module;
        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[replaced], Type::String);

        // RedCastElim should now eliminate the redundant Cast.
        let mut mb2 = ModuleBuilder::new("test");
        mb2.add_function(func.clone());
        let module2 = mb2.build();
        let result = crate::transforms::red_cast_elim::RedundantCastElimination
            .apply(module2)
            .unwrap();
        assert!(result.changed, "redundant coerce should be eliminated");
        let func = &result.module.functions[FuncId::new(0)];
        // After RedCastElim, the redundant cast is removed from blocks and all
        // uses of `coerced` are substituted with `replaced`.
        let live: std::collections::HashSet<_> = func
            .blocks
            .values()
            .flat_map(|b| b.insts.iter().copied())
            .collect();
        assert!(
            !live
                .iter()
                .any(|&id| func.insts[id].result == Some(coerced)),
            "Cast(String->String) should be eliminated entirely"
        );
    }
}
