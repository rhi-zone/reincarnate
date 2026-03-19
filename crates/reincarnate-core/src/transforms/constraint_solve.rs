use std::collections::HashMap;

use crate::error::CoreError;
use crate::ir::ty::{parse_type_notation, FunctionSig};
use crate::ir::{BlockId, Function, Module, Op, Type, ValueId};
use crate::pipeline::{Transform, TransformResult};

// ---------------------------------------------------------------------------
// Union-Find
// ---------------------------------------------------------------------------

/// A type variable index into the union-find.
type TVar = u32;

/// Flatten two concrete types into a union, absorbing `Unknown`.
fn make_union_type(a: Type, b: Type) -> Type {
    // Unknown absorbs everything.
    if matches!(a, Type::Unknown) || matches!(b, Type::Unknown) {
        return Type::Unknown;
    }
    // Flatten existing unions.
    let mut members: Vec<Type> = Vec::new();
    match a {
        Type::Union(vs) => members.extend(vs),
        t => members.push(t),
    }
    match b {
        Type::Union(vs) => {
            for v in vs {
                if !members.contains(&v) {
                    members.push(v);
                }
            }
        }
        t => {
            if !members.contains(&t) {
                members.push(t);
            }
        }
    }
    if members.len() == 1 {
        members.remove(0)
    } else {
        Type::Union(members)
    }
}

/// Union-find with path compression, union-by-rank, and optional type binding.
struct UnionFind {
    parent: Vec<u32>,
    rank: Vec<u8>,
    /// Concrete type bound to a representative, if any.
    resolved: Vec<Option<Type>>,
}

impl UnionFind {
    fn new() -> Self {
        Self {
            parent: Vec::new(),
            rank: Vec::new(),
            resolved: Vec::new(),
        }
    }

    /// Allocate a fresh unbound type variable.
    fn fresh(&mut self) -> TVar {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        self.rank.push(0);
        self.resolved.push(None);
        id
    }

    /// Allocate a fresh type variable pre-bound to a concrete type.
    fn fresh_with_type(&mut self, ty: Type) -> TVar {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        self.rank.push(0);
        self.resolved.push(Some(ty));
        id
    }

    /// Find the representative of `x` with path compression.
    fn find(&mut self, x: TVar) -> TVar {
        let mut root = x;
        while self.parent[root as usize] != root {
            root = self.parent[root as usize];
        }
        // Path compression.
        let mut cur = x;
        while cur != root {
            let next = self.parent[cur as usize];
            self.parent[cur as usize] = root;
            cur = next;
        }
        root
    }

    /// Unify two type variables. If both are bound to concrete types that
    /// differ, produces a `Type::Union` instead of failing.
    fn unify(&mut self, a: TVar, b: TVar) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }

        // Merge resolved types.
        let merged = match (
            self.resolved[ra as usize].take(),
            self.resolved[rb as usize].take(),
        ) {
            (Some(ta), Some(tb)) => {
                if ta == tb {
                    Some(ta)
                } else {
                    // Conflict — produce a union instead of an error.
                    Some(make_union_type(ta, tb))
                }
            }
            (Some(t), None) | (None, Some(t)) => Some(t),
            (None, None) => None,
        };

        // Union by rank.
        if self.rank[ra as usize] < self.rank[rb as usize] {
            self.parent[ra as usize] = rb;
            self.resolved[rb as usize] = merged;
        } else {
            self.parent[rb as usize] = ra;
            self.resolved[ra as usize] = merged;
            if self.rank[ra as usize] == self.rank[rb as usize] {
                self.rank[ra as usize] += 1;
            }
        }
    }

    /// Resolve a type variable to its concrete type, if bound.
    fn resolve(&mut self, var: TVar) -> Option<Type> {
        let r = self.find(var);
        self.resolved[r as usize].clone()
    }
}

// ---------------------------------------------------------------------------
// FunctionSolver
// ---------------------------------------------------------------------------

/// Pending constraint: the result of a `GetField` where the object was `Unknown`/`Unknown`.
struct HasFieldConstraint {
    object_var: TVar,
    field: String,
    result_var: TVar,
}

/// Pending constraint: a `CallIndirect` where the callee type was `Unknown`/`Unknown`.
struct CallableConstraint {
    callee_var: TVar,
    arg_vars: Vec<TVar>,
    result_var: Option<TVar>,
}

/// Per-function solver state: maps IR values to type variables and collects
/// equality constraints between them.
struct FunctionSolver {
    uf: UnionFind,
    value_vars: HashMap<ValueId, TVar>,
    constraints: Vec<(TVar, TVar)>,
    has_field: Vec<HasFieldConstraint>,
    callable: Vec<CallableConstraint>,
}

impl FunctionSolver {
    /// Build initial solver state from a function. Every value gets a type
    /// variable; concrete-typed values are pre-bound, `Unknown` and `Unknown`
    /// values are unbound (both are inference targets).
    fn from_function(func: &Function) -> Self {
        let mut uf = UnionFind::new();
        let mut value_vars = HashMap::new();

        for (vid, ty) in func.value_types.iter() {
            let var = if matches!(ty, Type::Unknown) {
                uf.fresh()
            } else {
                uf.fresh_with_type(ty.clone())
            };
            value_vars.insert(vid, var);
        }

        Self {
            uf,
            value_vars,
            constraints: Vec::new(),
            has_field: Vec::new(),
            callable: Vec::new(),
        }
    }

    /// Get the type variable for a value (must exist).
    fn var_for(&self, v: ValueId) -> TVar {
        self.value_vars[&v]
    }

    /// Add an equality constraint between two values.
    fn constrain_equal_values(&mut self, a: ValueId, b: ValueId) {
        let va = self.var_for(a);
        let vb = self.var_for(b);
        self.constraints.push((va, vb));
    }

    /// Constrain a value to a concrete type (allocates a bound temp var).
    fn constrain_value_to_type(&mut self, v: ValueId, ty: &Type) {
        if *ty == Type::Unknown {
            return;
        }
        let va = self.var_for(v);
        let tmp = self.uf.fresh_with_type(ty.clone());
        self.constraints.push((va, tmp));
    }
}

// ---------------------------------------------------------------------------
// ConstraintModuleContext
// ---------------------------------------------------------------------------

/// Module-level type context for the constraint solver.
/// Stores full `FunctionSig` (params + return) unlike the forward-only pass
/// which only stores return types.
struct ConstraintModuleContext {
    /// Function name → full signature.
    func_sigs: HashMap<String, FunctionSig>,
    /// Struct name → field name → field type.
    struct_fields: HashMap<String, HashMap<String, Type>>,
    /// (class_short_name, bare_method_name) → full signature.
    method_sigs: HashMap<(String, String), FunctionSig>,
    /// class_short_name → super_class_short_name.
    class_hierarchy: HashMap<String, Option<String>>,
    /// bare_method_name → full signature (only for unambiguous names).
    unique_method_sigs: HashMap<String, FunctionSig>,
}

impl ConstraintModuleContext {
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

        let mut func_sigs: HashMap<String, FunctionSig> = module
            .functions
            .iter()
            .map(|(id, f)| (module.func_name(id).to_string(), f.sig.clone()))
            .collect();

        // Extend with external function signatures from runtime.
        for (name, ext_sig) in &module.external_function_sigs {
            func_sigs
                .entry(name.clone())
                .or_insert_with(|| FunctionSig {
                    params: ext_sig
                        .params
                        .iter()
                        .map(|p| parse_type_notation(p))
                        .collect(),
                    return_ty: parse_type_notation(&ext_sig.returns),
                    ..Default::default()
                });
        }

        let mut method_sigs = HashMap::new();
        for (id, f) in module.functions.iter() {
            if f.class.is_some() {
                let fname = module.func_name(id);
                if let Some(bare) = fname.rsplit("::").next() {
                    if let Some(class) = &f.class {
                        method_sigs.insert((class.clone(), bare.to_string()), f.sig.clone());
                    }
                }
            }
        }

        let mut class_hierarchy: HashMap<String, Option<String>> = HashMap::new();
        for class in &module.classes {
            let super_short = class
                .super_class
                .as_ref()
                .map(|sc| sc.rsplit("::").next().unwrap_or(sc).to_string());
            class_hierarchy.insert(class.name.clone(), super_short);
        }

        // Extend with external type definitions from runtime.
        for (name, ext) in &module.external_type_defs {
            // class_hierarchy
            class_hierarchy
                .entry(name.clone())
                .or_insert_with(|| ext.extends.clone());
            // struct_fields
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
            // method_sigs: build FunctionSig from external method signatures
            for (method, ext_sig) in &ext.methods {
                method_sigs
                    .entry((name.clone(), method.clone()))
                    .or_insert_with(|| FunctionSig {
                        params: ext_sig
                            .params
                            .iter()
                            .map(|p| parse_type_notation(p))
                            .collect(),
                        return_ty: parse_type_notation(&ext_sig.returns),
                        ..Default::default()
                    });
            }
        }

        // Build unique_method_sigs: bare names that resolve to a single signature.
        let mut bare_name_sigs: HashMap<String, Option<FunctionSig>> = HashMap::new();
        for ((_, bare), sig) in &method_sigs {
            match bare_name_sigs.get(bare) {
                None => {
                    bare_name_sigs.insert(bare.clone(), Some(sig.clone()));
                }
                Some(Some(existing)) if *existing == *sig => {}
                Some(Some(_)) => {
                    bare_name_sigs.insert(bare.clone(), None);
                }
                Some(None) => {}
            }
        }
        let unique_method_sigs = bare_name_sigs
            .into_iter()
            .filter_map(|(name, sig)| sig.map(|s| (name, s)))
            .collect();

        Self {
            func_sigs,
            struct_fields,
            method_sigs,
            class_hierarchy,
            unique_method_sigs,
        }
    }

    /// Resolve the type of a field by walking the class hierarchy.
    fn resolve_field_type(&self, struct_name: &str, field: &str) -> Option<Type> {
        let mut current = Some(struct_name.to_string());
        while let Some(name) = current {
            if let Some(fields) = self.struct_fields.get(&name) {
                if let Some(ty) = fields.get(field) {
                    return Some(ty.clone());
                }
            }
            current = self.class_hierarchy.get(&name).and_then(|p| p.clone());
        }
        None
    }

    /// Resolve a method's signature by walking the class hierarchy, falling
    /// back to unique bare name. Same 3-strategy chain as `type_infer.rs`.
    fn resolve_func_sig(
        &self,
        name: &str,
        first_arg_ty: Option<&Type>,
        func_sig: &FunctionSig,
    ) -> Option<FunctionSig> {
        // Strategy 1: exact qualified name lookup.
        if let Some(sig) = self.func_sigs.get(name) {
            // Skip self-references (calling function would just return its own sig).
            if sig != func_sig || sig.params.iter().any(|p| *p != Type::Unknown) {
                return Some(sig.clone());
            }
        }

        let bare = name.rsplit("::").next().unwrap_or(name);

        // Strategy 2: receiver-based — if first arg is Struct(class), walk hierarchy.
        if let Some(Type::Struct(class)) = first_arg_ty {
            if let Some(sig) = self.resolve_method_sig(class, bare) {
                return Some(sig);
            }
        }

        // Strategy 3: unique bare name fallback.
        self.unique_method_sigs.get(bare).cloned()
    }

    /// Walk class hierarchy to find method signature.
    fn resolve_method_sig(&self, class: &str, method: &str) -> Option<FunctionSig> {
        let mut current = Some(class.to_string());
        let max_depth = self.class_hierarchy.len();
        for _ in 0..=max_depth {
            let Some(cls) = current else { break };
            if let Some(sig) = self.method_sigs.get(&(cls.clone(), method.to_string())) {
                return Some(sig.clone());
            }
            current = self.class_hierarchy.get(&cls).and_then(|s| s.clone());
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Constraint generation
// ---------------------------------------------------------------------------

/// Build a map from alloc ValueId → alloc type for Store constraints.
fn build_alloc_types_from_op(func: &Function) -> HashMap<ValueId, Type> {
    let mut alloc_types = HashMap::new();
    for block in func.blocks.values() {
        for &inst_id in &block.insts {
            let inst = &func.insts[inst_id];
            if let Op::Alloc(ty) = &inst.op {
                if let Some(result) = inst.result {
                    if *ty != Type::Unknown {
                        alloc_types.insert(result, ty.clone());
                    }
                }
            }
        }
    }
    alloc_types
}

/// Constrain branch arguments to equal target block parameters.
fn constrain_branch_args(
    solver: &mut FunctionSolver,
    func: &Function,
    target: BlockId,
    args: &[ValueId],
) {
    let params = &func.blocks[target].params;
    for (arg, param) in args.iter().zip(params.iter()) {
        solver.constrain_equal_values(*arg, param.value);
    }
}

/// Emit index/element type constraints for `GetIndex` and `SetIndex`.
///
/// - `collection`: the collection being indexed.
/// - `index`: the index value.
/// - `get_result`: the result of a `GetIndex`, if any.
/// - `set_value`: the value being written by a `SetIndex`, if any.
///
/// If the collection type is known, we constrain:
/// - Array(_): index must be Int(64); element type constrains result/value.
/// - Struct(_): index must be String (dynamic field access).
/// - Map(k, v): index must be k; element type constrains result/value.
fn constrain_index_op(
    solver: &mut FunctionSolver,
    func: &Function,
    collection: ValueId,
    index: ValueId,
    get_result: &Option<ValueId>,
    set_value: Option<ValueId>,
) {
    match &func.value_types[collection] {
        Type::Array(elem) => {
            solver.constrain_value_to_type(index, &Type::Int(64));
            let elem = elem.clone();
            if !matches!(*elem, Type::Unknown) {
                if let Some(r) = get_result {
                    solver.constrain_value_to_type(*r, &elem);
                }
                if let Some(v) = set_value {
                    solver.constrain_value_to_type(v, &elem);
                }
            }
        }
        Type::Struct(_) => {
            // Unknown field access: key must be a string.
            solver.constrain_value_to_type(index, &Type::String);
        }
        Type::Map(k, v) => {
            let k = k.clone();
            let v = v.clone();
            if !matches!(*k, Type::Unknown) {
                solver.constrain_value_to_type(index, &k);
            }
            if !matches!(*v, Type::Unknown) {
                if let Some(r) = get_result {
                    solver.constrain_value_to_type(*r, &v);
                }
                if let Some(sv) = set_value {
                    solver.constrain_value_to_type(sv, &v);
                }
            }
        }
        _ => {}
    }
}

/// Walk all instructions in block order and generate equality constraints.
fn generate_constraints(
    solver: &mut FunctionSolver,
    func: &Function,
    ctx: &ConstraintModuleContext,
) {
    let alloc_types = build_alloc_types_from_op(func);

    for block in func.blocks.values() {
        for &inst_id in &block.insts {
            let inst = &func.insts[inst_id];
            let result = inst.result;

            match &inst.op {
                // Arithmetic: a = b = r (no numeric ground — `+` is string concat
                // too; Sub/Mul/Div are always numeric but a value also used as a
                // GetIndex collection would get no counter-constraint and be wrongly
                // narrowed to Float(64), causing TS7053/TS2362).
                Op::Add(a, b) | Op::Sub(a, b) | Op::Mul(a, b) | Op::Div(a, b) | Op::Rem(a, b) => {
                    solver.constrain_equal_values(*a, *b);
                    if let Some(r) = result {
                        solver.constrain_equal_values(*a, r);
                    }
                }
                Op::Neg(a) => {
                    if let Some(r) = result {
                        solver.constrain_equal_values(*a, r);
                    }
                }

                // Bitwise: a = b = r (no integer ground for same reason as above).
                Op::BitAnd(a, b)
                | Op::BitOr(a, b)
                | Op::BitXor(a, b)
                | Op::Shl(a, b)
                | Op::Shr(a, b) => {
                    solver.constrain_equal_values(*a, *b);
                    if let Some(r) = result {
                        solver.constrain_equal_values(*a, r);
                    }
                }
                Op::BitNot(a) => {
                    if let Some(r) = result {
                        solver.constrain_equal_values(*a, r);
                    }
                }

                // Comparison: a = b, r = Bool
                Op::Cmp(_, a, b) => {
                    solver.constrain_equal_values(*a, *b);
                    if let Some(r) = result {
                        solver.constrain_value_to_type(r, &Type::Bool);
                    }
                }

                // Not: a = Bool, r = Bool
                Op::Not(a) => {
                    solver.constrain_value_to_type(*a, &Type::Bool);
                    if let Some(r) = result {
                        solver.constrain_value_to_type(r, &Type::Bool);
                    }
                }

                // BoolAnd/BoolOr: a = Bool, b = Bool, r = Bool
                Op::BoolAnd(a, b) | Op::BoolOr(a, b) => {
                    solver.constrain_value_to_type(*a, &Type::Bool);
                    solver.constrain_value_to_type(*b, &Type::Bool);
                    if let Some(r) = result {
                        solver.constrain_value_to_type(r, &Type::Bool);
                    }
                }

                // Select: cond = Bool, on_true = on_false, on_true = r
                Op::Select {
                    cond,
                    on_true,
                    on_false,
                } => {
                    solver.constrain_value_to_type(*cond, &Type::Bool);
                    solver.constrain_equal_values(*on_true, *on_false);
                    if let Some(r) = result {
                        solver.constrain_equal_values(*on_true, r);
                    }
                }

                // Call: arg[i] = sig.params[i], r = sig.return_ty
                Op::Call { func: name, args } => {
                    let first_arg_ty = args.first().map(|v| &func.value_types[*v]);
                    if let Some(sig) = ctx.resolve_func_sig(name, first_arg_ty, &func.sig) {
                        for (arg, param_ty) in args.iter().zip(sig.params.iter()) {
                            solver.constrain_value_to_type(*arg, param_ty);
                        }
                        if let Some(r) = result {
                            solver.constrain_value_to_type(r, &sig.return_ty);
                        }
                    }
                }

                // Spread: v = r
                Op::Spread(v) => {
                    if let Some(r) = result {
                        solver.constrain_equal_values(*v, r);
                    }
                }

                // Cast: r = ty (no constraint on source)
                Op::Cast(_, ty, _) => {
                    if let Some(r) = result {
                        solver.constrain_value_to_type(r, ty);
                    }
                }

                // TypeCheck: r = Bool
                Op::TypeCheck(..) => {
                    if let Some(r) = result {
                        solver.constrain_value_to_type(r, &Type::Bool);
                    }
                }

                // SetField: value = field_ty (if struct type known)
                Op::SetField {
                    object,
                    field,
                    value,
                } => {
                    if let Type::Struct(name) = &func.value_types[*object] {
                        if let Some(field_ty) = ctx
                            .struct_fields
                            .get(name)
                            .and_then(|fields| fields.get(field))
                        {
                            solver.constrain_value_to_type(*value, field_ty);
                        }
                    }
                }

                // GetField: r = field_ty (if struct type known);
                // emit HasField pending constraint when object is Unknown.
                Op::GetField { object, field } => match &func.value_types[*object] {
                    Type::Struct(name) => {
                        if let Some(r) = result {
                            if let Some(field_ty) = ctx
                                .struct_fields
                                .get(name)
                                .and_then(|fields| fields.get(field))
                            {
                                solver.constrain_value_to_type(r, field_ty);
                            }
                        }
                    }
                    Type::Unknown => {
                        if let Some(r) = result {
                            solver.has_field.push(HasFieldConstraint {
                                object_var: solver.var_for(*object),
                                field: field.clone(),
                                result_var: solver.var_for(r),
                            });
                        }
                    }
                    _ => {}
                },

                // StructInit: r = Struct(name), field values = field types
                Op::StructInit { name, fields } => {
                    if let Some(r) = result {
                        solver.constrain_value_to_type(r, &Type::Struct(name.clone()));
                    }
                    if let Some(field_defs) = ctx.struct_fields.get(name) {
                        for (fname, fval) in fields {
                            if let Some(fty) = field_defs.get(fname) {
                                solver.constrain_value_to_type(*fval, fty);
                            }
                        }
                    }
                }

                // ArrayInit: all elems[i] = elems[0]
                Op::ArrayInit(elems) => {
                    if elems.len() > 1 {
                        let first = elems[0];
                        for elem in &elems[1..] {
                            solver.constrain_equal_values(first, *elem);
                        }
                    }
                }

                // Store: if Alloc(ty) has ty != Unknown, value = ty
                Op::Store { ptr, value } => {
                    if let Some(alloc_ty) = alloc_types.get(ptr) {
                        solver.constrain_value_to_type(*value, alloc_ty);
                    }
                }

                // MethodCall: constrain args to sig params if resolvable.
                Op::MethodCall {
                    receiver,
                    method: name,
                    args,
                } => {
                    let receiver_ty = Some(&func.value_types[*receiver]);
                    if let Some(sig) = ctx.resolve_func_sig(name, receiver_ty, &func.sig) {
                        // Skip the first param (receiver) in the signature.
                        for (arg, param_ty) in args.iter().zip(sig.params.iter().skip(1)) {
                            solver.constrain_value_to_type(*arg, param_ty);
                        }
                        if let Some(r) = result {
                            solver.constrain_value_to_type(r, &sig.return_ty);
                        }
                    }
                }

                // GetIndex: constrain index and result types from collection type.
                Op::GetIndex { collection, index } => {
                    constrain_index_op(solver, func, *collection, *index, &result, None);
                }

                // SetIndex: constrain index and value types from collection type.
                Op::SetIndex {
                    collection,
                    index,
                    value,
                } => {
                    constrain_index_op(solver, func, *collection, *index, &None, Some(*value));
                }

                // CallIndirect: emit Callable pending constraint when callee is Unknown.
                Op::CallIndirect { callee, args } => match &func.value_types[*callee] {
                    Type::Function(sig) => {
                        let sig = sig.clone();
                        for (arg, param_ty) in args.iter().zip(sig.params.iter()) {
                            if !matches!(param_ty, Type::Unknown) {
                                solver.constrain_value_to_type(*arg, param_ty);
                            }
                        }
                        if let Some(r) = result {
                            if !matches!(sig.return_ty, Type::Unknown | Type::Void) {
                                solver.constrain_value_to_type(r, &sig.return_ty);
                            }
                        }
                    }
                    Type::Unknown => {
                        solver.callable.push(CallableConstraint {
                            callee_var: solver.var_for(*callee),
                            arg_vars: args.iter().map(|v| solver.var_for(*v)).collect(),
                            result_var: result.map(|r| solver.var_for(r)),
                        });
                    }
                    _ => {}
                },

                // No additional constraints for these:
                Op::Const(_)
                | Op::Load(_)
                | Op::GlobalRef(_)
                | Op::SystemCall { .. }
                | Op::Alloc(_)
                | Op::TupleInit(_)
                | Op::Yield(_)
                | Op::CoroutineCreate { .. }
                | Op::CoroutineResume(_)
                | Op::MakeClosure { .. } => {}
            }
        }
    }

    // Constrain terminators: Return, BrIf cond, branch args.
    use crate::ir::inst::Terminator;
    for (_, block) in func.blocks.iter() {
        match &block.terminator {
            Terminator::Return(Some(v)) => {
                solver.constrain_value_to_type(*v, &func.sig.return_ty);
            }
            Terminator::BrIf {
                cond,
                then_target,
                then_args,
                else_target,
                else_args,
            } => {
                solver.constrain_value_to_type(*cond, &Type::Bool);
                constrain_branch_args(solver, func, *then_target, then_args);
                constrain_branch_args(solver, func, *else_target, else_args);
            }
            Terminator::Br { target, args } => {
                constrain_branch_args(solver, func, *target, args);
            }
            Terminator::Switch { cases, default, .. } => {
                for (_, target, args) in cases {
                    constrain_branch_args(solver, func, *target, args);
                }
                constrain_branch_args(solver, func, default.0, &default.1);
            }
            Terminator::Return(None) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Solve & apply
// ---------------------------------------------------------------------------

/// Run the constraint solver on a single function. Returns true if any types
/// were refined.
fn solve_function(func: &mut Function, ctx: &ConstraintModuleContext) -> bool {
    let mut solver = FunctionSolver::from_function(func);
    generate_constraints(&mut solver, func, ctx);

    // Solve: iterate equality constraints and unify.
    for (a, b) in solver.constraints.clone() {
        solver.uf.unify(a, b);
    }

    // Process pending HasField constraints: if the object var has resolved to a
    // Struct, look up the field type and constrain the result var.
    let has_field = std::mem::take(&mut solver.has_field);
    for hf in &has_field {
        if let Some(Type::Struct(name)) = solver.uf.resolve(hf.object_var) {
            if let Some(field_ty) = ctx.resolve_field_type(&name, &hf.field) {
                let tmp = solver.uf.fresh_with_type(field_ty);
                solver.uf.unify(hf.result_var, tmp);
            }
        }
    }

    // Process pending Callable constraints: if the callee var has resolved to a
    // Function type, constrain arg vars and result var from the signature.
    let callable = std::mem::take(&mut solver.callable);
    for cc in &callable {
        if let Some(Type::Function(sig)) = solver.uf.resolve(cc.callee_var) {
            for (arg_var, param_ty) in cc.arg_vars.iter().zip(sig.params.iter()) {
                if !matches!(param_ty, Type::Unknown) {
                    let tmp = solver.uf.fresh_with_type(param_ty.clone());
                    solver.uf.unify(*arg_var, tmp);
                }
            }
            if let Some(result_var) = cc.result_var {
                if !matches!(sig.return_ty, Type::Unknown | Type::Void) {
                    let tmp = solver.uf.fresh_with_type(sig.return_ty.clone());
                    solver.uf.unify(result_var, tmp);
                }
            }
        }
    }

    // Collect updates: Unknown or Unknown values that now have concrete types.
    let mut changed = false;
    let updates: Vec<(ValueId, Type)> = solver
        .value_vars
        .iter()
        .filter_map(|(vid, &var)| {
            if !matches!(func.value_types[*vid], Type::Unknown) {
                return None;
            }
            let ty = solver.uf.resolve(var)?;
            if matches!(ty, Type::Unknown) {
                return None;
            }
            Some((*vid, ty))
        })
        .collect();

    for (vid, ty) in updates {
        func.value_types[vid] = ty;
        changed = true;
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
            changed = true;
        }
    }

    changed
}

// ---------------------------------------------------------------------------
// Transform impl
// ---------------------------------------------------------------------------

/// Constraint-based type inference pass. Runs after `TypeInference` to refine
/// remaining `Unknown` types via unification of equality constraints.
pub struct ConstraintSolve;

impl Transform for ConstraintSolve {
    fn name(&self) -> &str {
        "constraint-solve"
    }

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        let ctx = ConstraintModuleContext::from_module(&module);
        let mut changed = false;
        for func_id in module.functions.keys().collect::<Vec<_>>() {
            changed |= solve_function(&mut module.functions[func_id], &ctx);
        }
        Ok(TransformResult { module, changed })
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
    use crate::ir::ty::FunctionSig;
    use crate::ir::{ClassDef, FieldDef, FuncId, StructDef, Visibility};

    // ---- Identity & idempotency tests ----

    /// No type variables, all concrete → no changes.
    #[test]
    fn identity_no_change() {
        let sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0);
        fb.ret(Some(p));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        let result = ConstraintSolve.apply(module).unwrap();
        assert!(!result.changed);
    }

    /// Constraint solving is idempotent.
    #[test]
    fn idempotent_after_transform() {
        let callee_sig = FunctionSig {
            params: vec![Type::Int(32)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee_fb = FunctionBuilder::new("foo", callee_sig, Visibility::Private);
        callee_fb.ret(None);
        let callee = callee_fb.build();

        let caller_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller_fb = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        let arg = caller_fb.param(0);
        caller_fb.call("foo", &[arg], Type::Void);
        caller_fb.ret(None);
        let caller = caller_fb.build();

        // Build module with both functions for idempotency test.
        let mut mb = ModuleBuilder::new("test");
        mb.add_function(callee);
        mb.add_function(caller);
        let module = mb.build();
        let r1 = ConstraintSolve.apply(module).unwrap();
        assert!(r1.changed);
        let r2 = ConstraintSolve.apply(r1.module).unwrap();
        assert!(!r2.changed);
    }

    // -- UnionFind unit tests --

    #[test]
    fn union_find_basic_unify() {
        let mut uf = UnionFind::new();
        let a = uf.fresh_with_type(Type::Int(32));
        let b = uf.fresh();
        uf.unify(a, b);
        assert_eq!(uf.resolve(b), Some(Type::Int(32)));
    }

    #[test]
    fn union_find_path_compression() {
        let mut uf = UnionFind::new();
        let a = uf.fresh_with_type(Type::Bool);
        let b = uf.fresh();
        let c = uf.fresh();
        let d = uf.fresh();
        uf.unify(a, b);
        uf.unify(b, c);
        uf.unify(c, d);
        // After find, d should resolve through compressed path.
        assert_eq!(uf.resolve(d), Some(Type::Bool));
    }

    #[test]
    fn union_find_conflict_produces_union() {
        // Unifying two vars with different concrete types now produces a Union.
        let mut uf = UnionFind::new();
        let a = uf.fresh_with_type(Type::Int(32));
        let b = uf.fresh_with_type(Type::String);
        uf.unify(a, b);
        // The representative should now hold a union of both types.
        let resolved = uf.resolve(a).expect("should be resolved");
        match resolved {
            Type::Union(members) => {
                assert!(members.contains(&Type::Int(32)));
                assert!(members.contains(&Type::String));
                assert_eq!(members.len(), 2);
            }
            other => panic!("expected Type::Union, got {:?}", other),
        }
    }

    #[test]
    fn union_find_same_type_ok() {
        let mut uf = UnionFind::new();
        let a = uf.fresh_with_type(Type::Int(64));
        let b = uf.fresh_with_type(Type::Int(64));
        uf.unify(a, b);
        assert_eq!(uf.resolve(a), Some(Type::Int(64)));
    }

    // -- Integration tests --

    #[test]
    fn call_arg_backward_flow() {
        // foo(x: Int(32)) → Void. Caller passes Unknown arg → should refine.
        let callee_sig = FunctionSig {
            params: vec![Type::Int(32)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee_fb = FunctionBuilder::new("foo", callee_sig, Visibility::Private);
        callee_fb.ret(None);
        let callee = callee_fb.build();

        let caller_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller_fb = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        let arg = caller_fb.param(0);
        caller_fb.call("foo", &[arg], Type::Void);
        caller_fb.ret(None);
        let caller = caller_fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(callee);
        mb.add_function(caller);
        let module = mb.build();

        let transform = ConstraintSolve;
        let module = transform.apply(module).unwrap().module;

        let caller_func = &module.functions[FuncId::new(1)];
        assert_eq!(caller_func.value_types[arg], Type::Int(32));
    }

    #[test]
    fn return_backward_flow() {
        // fn returning Int(32), returns a Unknown param → param should refine.
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Int(32),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0);
        fb.ret(Some(p));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = ConstraintSolve;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[p], Type::Int(32));
    }

    #[test]
    fn arithmetic_equalization() {
        // Add(v_dynamic, v_int64) → v_dynamic should become Int(64).
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let a = fb.param(0); // Unknown
        let b = fb.const_int(42); // Int(64)
        let sum = fb.add(a, b);
        fb.ret(Some(sum));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = ConstraintSolve;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[a], Type::Int(64));
    }

    #[test]
    fn brif_cond_refined_to_bool() {
        // BrIf on a Unknown cond → should become Bool.
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let cond = fb.param(0); // Unknown
        let then_b = fb.create_block();
        let else_b = fb.create_block();
        fb.br_if(cond, then_b, &[], else_b, &[]);

        fb.switch_to_block(then_b);
        fb.ret(None);
        fb.switch_to_block(else_b);
        fb.ret(None);
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = ConstraintSolve;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[cond], Type::Bool);
    }

    #[test]
    fn transitive_constraint_flow() {
        // v1 = param(Unknown), v2 = copy(v1), call("foo", [v2]) where foo
        // expects Int(32) → both v1 and v2 should become Int(32).
        let callee_sig = FunctionSig {
            params: vec![Type::Int(32)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee_fb = FunctionBuilder::new("foo", callee_sig, Visibility::Private);
        callee_fb.ret(None);
        let callee = callee_fb.build();

        let caller_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller_fb = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        let v1 = caller_fb.param(0);
        caller_fb.call("foo", &[v1], Type::Void);
        caller_fb.ret(None);
        let caller = caller_fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(callee);
        mb.add_function(caller);
        let module = mb.build();

        let transform = ConstraintSolve;
        let module = transform.apply(module).unwrap().module;

        let caller_func = &module.functions[FuncId::new(1)];
        assert_eq!(caller_func.value_types[v1], Type::Int(32));
    }

    #[test]
    fn conflict_produces_union() {
        // Value constrained to both Int(32) (by call) and String (by return) → produces Union.
        let callee_sig = FunctionSig {
            params: vec![Type::Int(32)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee_fb = FunctionBuilder::new("foo", callee_sig, Visibility::Private);
        callee_fb.ret(None);
        let callee = callee_fb.build();

        let caller_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::String,
            ..Default::default()
        };
        let mut caller_fb = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        let p = caller_fb.param(0);
        caller_fb.call("foo", &[p], Type::Void);
        caller_fb.ret(Some(p));
        let caller = caller_fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(callee);
        mb.add_function(caller);
        let module = mb.build();

        let transform = ConstraintSolve;
        let module = transform.apply(module).unwrap().module;

        let caller_func = &module.functions[FuncId::new(1)];
        // Conflict → Union instead of Unknown.
        match &caller_func.value_types[p] {
            Type::Union(members) => {
                assert!(members.contains(&Type::Int(32)));
                assert!(members.contains(&Type::String));
            }
            other => panic!("expected Type::Union, got {:?}", other),
        }
    }

    #[test]
    fn concrete_values_preserved() {
        // Already-typed values should remain unchanged.
        let sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0); // Int(64)
        let c = fb.const_int(42); // Int(64)
        let sum = fb.add(p, c);
        fb.ret(Some(sum));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = ConstraintSolve;
        let result = transform.apply(module).unwrap();

        let func = &result.module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[p], Type::Int(64));
        assert_eq!(func.value_types[c], Type::Int(64));
        assert_eq!(func.value_types[sum], Type::Int(64));
        assert!(!result.changed);
    }

    #[test]
    fn block_param_from_branch() {
        // Branch with concrete arg → target block param should be refined.
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let cond = fb.param(0);

        let (merge, merge_vals) = fb.create_block_with_params(&[Type::Unknown]);
        let then_b = fb.create_block();
        let else_b = fb.create_block();
        fb.br_if(cond, then_b, &[], else_b, &[]);

        fb.switch_to_block(then_b);
        let a = fb.const_int(1);
        fb.br(merge, &[a]);

        fb.switch_to_block(else_b);
        let b = fb.const_int(2);
        fb.br(merge, &[b]);

        fb.switch_to_block(merge);
        fb.ret(Some(merge_vals[0]));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = ConstraintSolve;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[merge_vals[0]], Type::Int(64));
        assert_eq!(func.blocks[merge].params[0].ty, Type::Int(64));
    }

    #[test]
    fn noop_on_fully_typed() {
        // Module with no Unknown values → changed = false.
        let sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0);
        fb.ret(Some(p));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = ConstraintSolve;
        let result = transform.apply(module).unwrap();
        assert!(!result.changed);
    }

    #[test]
    fn method_call_backward_flow() {
        // Method call: Creature::isAlive(self: Struct("Creature")) → Bool.
        // Caller calls "isAlive" with Unknown receiver and uses result as Unknown.
        // → receiver should stay Unknown (no constraint on receiver type), but
        //   result should become Bool via unique method sig fallback.
        let method_sig = FunctionSig {
            params: vec![Type::Struct("Creature".to_string())],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut method_fb =
            FunctionBuilder::new("Creature::isAlive", method_sig, Visibility::Public);
        let self_param = method_fb.param(0);
        method_fb.ret(Some(self_param));
        let mut method_func = method_fb.build();
        method_func.class = Some("Creature".to_string());

        let caller_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut caller_fb = FunctionBuilder::new("caller", caller_sig, Visibility::Public);
        let recv = caller_fb.param(0);
        let result = caller_fb.call("isAlive", &[recv], Type::Unknown);
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

        let transform = ConstraintSolve;
        let module = transform.apply(module).unwrap().module;

        let caller = &module.functions[FuncId::new(1)];
        // Result gets Bool from method sig's return type.
        assert_eq!(caller.value_types[result], Type::Bool);
        // Receiver gets Struct("Creature") from method sig's param type.
        assert_eq!(
            caller.value_types[recv],
            Type::Struct("Creature".to_string())
        );
    }

    // ---- Edge case tests ----

    /// No type variables (all concrete) → unchanged.
    #[test]
    fn no_type_vars_noop() {
        let sig = FunctionSig {
            params: vec![Type::Int(64), Type::Bool],
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
        let result = ConstraintSolve.apply(module).unwrap();
        assert!(!result.changed);
    }

    /// Conflicting constraints (Int vs String) → produces Union.
    #[test]
    fn conflicting_constraints_produce_union() {
        let callee_int = FunctionSig {
            params: vec![Type::Int(32)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb1 = FunctionBuilder::new("want_int", callee_int, Visibility::Private);
        fb1.ret(None);
        let callee1 = fb1.build();

        let caller_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::String,
            ..Default::default()
        };
        let mut fb2 = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        let p = fb2.param(0);
        fb2.call("want_int", &[p], Type::Void);
        fb2.ret(Some(p)); // return type is String
        let caller = fb2.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(callee1);
        mb.add_function(caller);
        let module = mb.build();
        let result = ConstraintSolve.apply(module).unwrap();
        let func = &result.module.functions[FuncId::new(1)];
        match &func.value_types[p] {
            Type::Union(members) => {
                assert!(
                    members.contains(&Type::Int(32)) && members.contains(&Type::String),
                    "conflicting constraints → Union([Int(32), String])"
                );
            }
            other => panic!("expected Type::Union, got {:?}", other),
        }
    }

    // ---- Adversarial tests ----

    /// Spread chain propagation: Spread creates an Equal constraint between
    /// source and result. Return type should propagate through spread chain.
    #[test]
    fn spread_chain_propagation() {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let a = fb.param(0);
        let b = fb.spread(a);
        let c = fb.spread(b);
        fb.ret(Some(c));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        let result = ConstraintSolve.apply(module).unwrap();
        let func = &result.module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[a], Type::Int(64));
        assert_eq!(func.value_types[b], Type::Int(64));
        assert_eq!(func.value_types[c], Type::Int(64));
    }

    /// HasField pending constraint: object is Unknown at GetField but another
    /// instruction constrains it to Struct("Foo") → result should get field type.
    #[test]
    fn has_field_pending_resolved_via_equality() {
        // fn test(obj: Unknown, v: Unknown) -> Unknown
        //   r = obj.x      (Unknown object → HasField pending)
        //   call("set_foo", [obj])  (constrains obj to Struct("Foo"))
        //   return r
        let callee_sig = FunctionSig {
            params: vec![Type::Struct("Foo".to_string())],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee_fb = FunctionBuilder::new("set_foo", callee_sig, Visibility::Private);
        callee_fb.ret(None);
        let callee = callee_fb.build();

        let caller_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut caller_fb = FunctionBuilder::new("test", caller_sig, Visibility::Private);
        let obj = caller_fb.param(0); // Unknown
        let r = caller_fb.get_field(obj, "x", Type::Unknown);
        caller_fb.call("set_foo", &[obj], Type::Void);
        caller_fb.ret(Some(r));
        let caller = caller_fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_struct(StructDef {
            name: "Foo".into(),
            namespace: Vec::new(),
            fields: vec![FieldDef {
                name: "x".into(),
                ty: Type::Int(32),
                default: None,
            }],
            visibility: Visibility::Public,
        });
        mb.add_function(callee);
        mb.add_function(caller);
        let module = mb.build();

        let transform = ConstraintSolve;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(1)];
        // obj was narrowed to Struct("Foo") by the call constraint.
        assert_eq!(func.value_types[obj], Type::Struct("Foo".to_string()));
        // r should have been resolved to Int(32) via the HasField pending constraint.
        assert_eq!(func.value_types[r], Type::Int(32));
    }

    /// CallIndirect with a Unknown callee → no constraint applied to args (no panic).
    #[test]
    fn call_indirect_dynamic_callee_no_constraint() {
        // fn test(callee: Unknown, arg: Unknown) -> Unknown
        //   r = call_indirect callee(arg)
        //   return r
        let sig = FunctionSig {
            params: vec![Type::Unknown, Type::Unknown],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let callee = fb.param(0);
        let arg = fb.param(1);
        let r = fb.call_indirect(callee, &[arg], Type::Unknown);
        fb.ret(Some(r));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let transform = ConstraintSolve;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        // No constraints applied → all remain Unknown.
        assert_eq!(func.value_types[callee], Type::Unknown);
        assert_eq!(func.value_types[arg], Type::Unknown);
        assert_eq!(func.value_types[r], Type::Unknown);
    }

    #[test]
    fn set_field_constrains_value() {
        // SetField on a known struct constrains the value to the field type.
        let sig = FunctionSig {
            params: vec![Type::Struct("Point".to_string()), Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let obj = fb.param(0);
        let val = fb.param(1); // Unknown
        fb.set_field(obj, "x", val);
        fb.ret(None);
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

        let transform = ConstraintSolve;
        let module = transform.apply(module).unwrap().module;

        let func = &module.functions[FuncId::new(0)];
        assert_eq!(func.value_types[val], Type::Int(64));
    }
}
