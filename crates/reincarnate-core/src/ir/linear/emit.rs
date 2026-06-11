//! Phase 3: emit — LinearStmt -> Vec<Stmt>
//!
//! Converts LinearStmt sequences into the final AST (Vec<Stmt>) by building
//! expressions, inlining single-use values, and managing variable declarations.

use std::collections::{HashMap, HashSet};

use super::resolve::ResolveCtx;
use super::{
    is_deferrable, is_js_ident, is_memory_write, is_side_effecting_op, merge_coalesce_type,
    strip_trailing_continue, types_coalesce_compatible, LinearStmt,
};
use crate::entity::{EntityRef, PrimaryMap};
use crate::ir::ast::{Expr, Stmt};
use crate::ir::block::BlockId;
use crate::ir::func::{Function, MethodKind};
use crate::ir::inst::{CastKind, InstId, Op, Terminator};
use crate::ir::module::TypeDecl;
use crate::ir::ty::{FunctionSig, Type, TypeId};
use crate::ir::value::{Constant, ValueId};
use crate::pipeline::LoweringConfig;
use crate::transforms::util::value_operands;

// -----------------------------------------------------------------------
// Emit context
// -----------------------------------------------------------------------

pub(super) struct EmitCtx<'a> {
    func: &'a Function,
    config: &'a LoweringConfig,
    resolve: &'a ResolveCtx,
    /// Named type arena from the module — used for resolving TypeId → name.
    module_types: &'a PrimaryMap<TypeId, TypeDecl>,
    /// Debug names for values (func.value_names + out-of-SSA coalescing).
    value_names: HashMap<ValueId, String>,
    /// Entry-block parameter ValueIds.
    entry_params: HashSet<ValueId>,
    /// Names shared by 2+ ValueIds (out-of-SSA coalesced variables).
    shared_names: HashSet<String>,
    /// Widened declaration type for coalesced block-param names.
    /// When multiple non-entry block params share a name but have different
    /// IR types (e.g. `TimeModel` in one branch, `DefaultDict` in another),
    /// the declaration must use `Unknown` to avoid TS2739/TS2322.
    coalesced_decl_types: HashMap<String, Type>,
    /// Types for names hoisted from cross-scope SE inlines (used by
    /// collect_block_param_decls to emit `let name!: ty;` with the definite
    /// assignment assertion so TypeScript doesn't flag TS2454).
    cross_scope_hoisted_types: HashMap<String, Type>,
    /// Deferred single-use pure instructions (from Phase 2's lazy_inlines).
    pending_lazy: HashMap<ValueId, InstId>,
    /// Always-rebuild instructions (from Phase 2's always_inlines).
    always_inline_map: HashMap<ValueId, InstId>,
    /// Deferred side-effecting single-use expressions.
    side_effecting_inlines: HashMap<ValueId, Expr>,
    /// Values already declared by flush_side_effecting_inlines.
    se_flush_declared: HashSet<ValueId>,
    /// Values already materialized by emit_or_inline (count >= 2 path).
    /// Prevents build_val from inserting them into referenced_block_params,
    /// which would cause collect_block_param_decls to emit a duplicate `let`.
    or_inline_declared: HashSet<ValueId>,
    /// Block-param ValueIds referenced during emission.
    referenced_block_params: HashSet<ValueId>,
    /// All non-entry block-param ValueIds (for distinguishing true params from
    /// arbitrary values that fell through build_val).
    all_block_params: HashSet<ValueId>,
    /// Pending-lazy values protected from flush_pending_reads (header reads
    /// that shouldn't be flushed into nested bodies).
    flush_protected: HashSet<ValueId>,
}

impl<'a> EmitCtx<'a> {
    pub(super) fn new(
        func: &'a Function,
        resolve: &'a ResolveCtx,
        config: &'a LoweringConfig,
        module_types: &'a PrimaryMap<TypeId, TypeDecl>,
    ) -> Self {
        let mut value_names: HashMap<ValueId, String> = func
            .value_names
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        let entry_params: HashSet<ValueId> = func.blocks[func.entry]
            .params
            .iter()
            .map(|p| p.value)
            .collect();

        // Out-of-SSA name coalescing: bidirectional propagation across branch edges.
        // Forward: named block param -> unnamed branch arg.
        // Reverse: when all named branch args agree -> unnamed block param.
        // Fixpoint: naming values in one direction may enable naming in the other.
        loop {
            let mut changed = false;

            // Forward: propagate block-param names to branch args.
            for (_, block) in func.blocks.iter() {
                let mut propagate_fwd = |target: BlockId, args: &[ValueId]| {
                    let target_block = &func.blocks[target];
                    for (param, &src) in target_block.params.iter().zip(args.iter()) {
                        if param.value == src || value_names.contains_key(&src) {
                            continue;
                        }
                        if let Some(name) = value_names.get(&param.value) {
                            let name = name.clone();
                            value_names.insert(src, name);
                            changed = true;
                        }
                    }
                };
                match &block.terminator {
                    Terminator::Br { target, args } => propagate_fwd(*target, args),
                    Terminator::BrIf {
                        then_target,
                        then_args,
                        else_target,
                        else_args,
                        ..
                    } => {
                        propagate_fwd(*then_target, then_args);
                        propagate_fwd(*else_target, else_args);
                    }
                    Terminator::Switch { cases, default, .. } => {
                        for (_, target, args) in cases {
                            propagate_fwd(*target, args);
                        }
                        propagate_fwd(default.0, &default.1);
                    }
                    Terminator::Return(_) => {}
                }
            }

            // Reverse: propagate branch-arg names -> unnamed block params.
            // Only assign when all named args feeding a param agree on the same name.
            let mut candidates: HashMap<ValueId, Option<String>> = HashMap::new();
            for (_, block) in func.blocks.iter() {
                let mut collect = |target: BlockId, args: &[ValueId]| {
                    let target_block = &func.blocks[target];
                    for (param, &src) in target_block.params.iter().zip(args.iter()) {
                        if value_names.contains_key(&param.value) {
                            continue;
                        }
                        if let Some(src_name) = value_names.get(&src) {
                            candidates
                                .entry(param.value)
                                .and_modify(|existing| {
                                    if let Some(prev) = existing {
                                        if prev != src_name {
                                            *existing = None; // conflict
                                        }
                                    }
                                })
                                .or_insert_with(|| Some(src_name.clone()));
                        }
                    }
                };
                match &block.terminator {
                    Terminator::Br { target, args } => collect(*target, args),
                    Terminator::BrIf {
                        then_target,
                        then_args,
                        else_target,
                        else_args,
                        ..
                    } => {
                        collect(*then_target, then_args);
                        collect(*else_target, else_args);
                    }
                    Terminator::Switch { cases, default, .. } => {
                        for (_, target, args) in cases {
                            collect(*target, args);
                        }
                        collect(default.0, &default.1);
                    }
                    Terminator::Return(_) => {}
                }
            }
            // Sort candidates by ValueId for deterministic fixpoint convergence.
            let mut sorted_candidates: Vec<_> = candidates.into_iter().collect();
            sorted_candidates.sort_by_key(|(v, _)| v.index());
            for (param_value, candidate) in sorted_candidates {
                if let Some(name) = candidate {
                    value_names.insert(param_value, name);
                    changed = true;
                }
            }

            if !changed {
                break;
            }
        }

        // Conflict resolution: when a non-local value (not one of this block's
        // own params) is used in a block instruction and shares its name with a
        // local block param, rename the non-local value to a synthetic "v{index}"
        // name.  This prevents two distinct SSA values from emitting as the same
        // identifier in the same expression.
        {
            let mut conflicts: Vec<ValueId> = Vec::new();
            for (_, block) in func.blocks.iter() {
                let local_param_values: HashSet<ValueId> =
                    block.params.iter().map(|p| p.value).collect();
                let local_names: HashSet<String> = block
                    .params
                    .iter()
                    .filter_map(|p| value_names.get(&p.value).cloned())
                    .collect();
                if local_names.is_empty() {
                    continue;
                }
                for &iid in &block.insts {
                    for v in value_operands(&func.insts[iid].op) {
                        if !local_param_values.contains(&v) {
                            if let Some(name) = value_names.get(&v) {
                                if local_names.contains(name) {
                                    conflicts.push(v);
                                }
                            }
                        }
                    }
                }
            }
            for v in conflicts {
                value_names.insert(v, format!("v{}", v.index()));
            }
        }

        // Propagate names from Cast/Copy results to their source operands.
        // Mem2Reg names the stored value (e.g. Cast(src)), but if the Cast
        // is single-use and gets lazily inlined, its name is never used.
        // Propagating to `src` ensures the materialized variable gets the name.
        // Track which sources received propagated names — these are excluded
        // from shared_names since they are absorbed (side-effecting inlines)
        // and never produce their own statements.
        // Track sources that received names AND will be absorbed (single-use
        // values become lazy or side-effecting inlines — never standalone stmts).
        // Block params always produce their own declarations via collect_block_param_decls
        // and must never be treated as "absorbed" even when use_count <= 1.
        let all_block_param_values: HashSet<ValueId> = func
            .blocks
            .iter()
            .flat_map(|(_, block)| block.params.iter().map(|p| p.value))
            .collect();

        let mut propagated_sources: HashSet<ValueId> = HashSet::new();
        for (_, inst) in func.insts.iter() {
            if let Some(result) = inst.result {
                if let Some(name) = value_names.get(&result).cloned() {
                    let src = match &inst.op {
                        Op::Cast(s, ..) => Some(*s),
                        _ => None,
                    };
                    if let Some(src) = src {
                        if let std::collections::hash_map::Entry::Vacant(e) = value_names.entry(src)
                        {
                            e.insert(name);
                            let src_uses = resolve.use_counts.get(&src).copied().unwrap_or(0);
                            // Block params are never absorbable: they always generate
                            // their own `let` declaration regardless of use count.
                            if src_uses <= 1 && !all_block_param_values.contains(&src) {
                                propagated_sources.insert(src);
                            }
                        }
                    }
                }
            }
        }

        // Deduplicate self-parameter name: values other than param 0 that share
        // the self name should use a distinct local name to avoid `this = ...`.
        let has_self = matches!(
            func.method_kind,
            MethodKind::Instance
                | MethodKind::Constructor
                | MethodKind::Getter
                | MethodKind::Setter
        );
        if has_self && !func.blocks[func.entry].params.is_empty() {
            let self_value = func.blocks[func.entry].params[0].value;
            if let Some(self_name) = value_names.get(&self_value).cloned() {
                let alt_name = format!("_{self_name}");
                for (vid, name) in value_names.iter_mut() {
                    if *vid != self_value && *name == self_name {
                        *name = alt_name.clone();
                    }
                }
            }
        }

        // Find names shared by 2+ ValueIds, excluding values whose names were
        // propagated from Cast/Copy results — those values are absorbed as
        // side-effecting inlines and never produce standalone statements.
        let mut name_counts: HashMap<&str, usize> = HashMap::new();
        for (vid, name) in &value_names {
            if propagated_sources.contains(vid) {
                continue;
            }
            *name_counts.entry(name.as_str()).or_default() += 1;
        }
        let mut shared_names: HashSet<String> = name_counts
            .into_iter()
            .filter(|(_, count)| *count >= 2)
            .map(|(name, _)| name.to_string())
            .collect();

        // Compute widened declaration types for all coalesced (shared) names.
        // When multiple values share a name but have different IR types, widen
        // the declared type to Unknown so TypeScript doesn't flag TS2739/TS2322
        // on assignments from a branch with a different type.  Covers both
        // block params AND instruction results that end up sharing a name.
        let mut coalesced_decl_types: HashMap<String, Type> = {
            let mut name_types: HashMap<String, Type> = HashMap::new();
            for (vid, name) in &value_names {
                if !shared_names.contains(name.as_str()) {
                    continue;
                }
                if propagated_sources.contains(vid) {
                    continue; // absorbed side-effecting inlines — never standalone stmts
                }
                if func.null_sentinel_values.contains(vid) {
                    continue; // sentinels are never emitted as standalone stmts
                }
                let ty = func.value_types[*vid].clone();
                name_types
                    .entry(name.clone())
                    .and_modify(|existing| {
                        *existing = merge_coalesce_type(existing.clone(), ty.clone());
                    })
                    .or_insert(ty);
            }
            name_types
        };

        let all_block_params: HashSet<ValueId> = func
            .blocks
            .iter()
            .filter(|(bid, _)| *bid != func.entry)
            .flat_map(|(_, block)| block.params.iter().map(|p| p.value))
            .collect();

        // Detect instruction results defined in loop bodies but used outside.
        // These need function-scope `let` declarations because the linearizer
        // scopes their `const` inside the while body, making them invisible
        // after the loop (TS2304).  We add their names to shared_names so the
        // existing hoisting mechanism emits `let name;` + Assign inside the loop.
        {
            let mut loop_blocks: HashSet<BlockId> = HashSet::new();
            for (block_id, block) in func.blocks.iter() {
                let targets = crate::transforms::util::branch_targets(&block.terminator);
                for t in &targets {
                    if *t == block_id {
                        loop_blocks.insert(block_id);
                    }
                }
            }

            if !loop_blocks.is_empty() {
                let mut def_block: HashMap<ValueId, BlockId> = HashMap::new();
                for (block_id, block) in func.blocks.iter() {
                    for &inst_id in &block.insts {
                        if let Some(r) = func.insts[inst_id].result {
                            def_block.insert(r, block_id);
                        }
                    }
                }

                for (block_id, block) in func.blocks.iter() {
                    for &inst_id in &block.insts {
                        let op = &func.insts[inst_id].op;
                        for operand in value_operands(op) {
                            if let Some(&db) = def_block.get(&operand) {
                                if loop_blocks.contains(&db) && db != block_id {
                                    let name = value_names
                                        .get(&operand)
                                        .cloned()
                                        .unwrap_or_else(|| format!("v{}", operand.index()));
                                    shared_names.insert(name.clone());
                                    // Record the type for the hoisted declaration.
                                    coalesced_decl_types
                                        .entry(name)
                                        .or_insert_with(|| func.value_types[operand].clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Pre-populate always_inline_map for every always-inline value in the
        // function.  Without this, a use in Dispatch case A of a value whose
        // Def instruction is in Dispatch case B (processed later) would fall
        // through to Expr::Var(name) with no declaration → TS2304.  Pre-populating
        // ensures build_val can always rebuild the expression regardless of order.
        let always_inline_map: HashMap<ValueId, InstId> = func
            .insts
            .iter()
            .filter_map(|(inst_id, inst)| {
                let result = inst.result?;
                if resolve.always_inlines.contains(&result) {
                    Some((result, inst_id))
                } else {
                    None
                }
            })
            .collect();

        Self {
            func,
            config,
            resolve,
            module_types,
            value_names,
            entry_params,
            shared_names,
            coalesced_decl_types,
            cross_scope_hoisted_types: HashMap::new(),
            pending_lazy: HashMap::new(),
            always_inline_map,
            side_effecting_inlines: HashMap::new(),
            se_flush_declared: HashSet::new(),
            or_inline_declared: HashSet::new(),
            referenced_block_params: HashSet::new(),
            all_block_params,
            flush_protected: HashSet::new(),
        }
    }

    /// Resolve a TypeId to its name string.
    ///
    /// Returns an empty string when the TypeId is not in the arena (e.g. in
    /// unit tests where `module_types` is empty).
    fn type_name(&self, id: TypeId) -> &str {
        self.module_types
            .get(id)
            .and_then(|t| t.name())
            .unwrap_or("")
    }

    fn value_name(&self, v: ValueId) -> String {
        if let Some(name) = self.value_names.get(&v) {
            name.clone()
        } else {
            format!("v{}", v.index())
        }
    }

    fn use_count(&self, v: ValueId) -> usize {
        self.resolve.use_counts.get(&v).copied().unwrap_or(0)
    }

    /// Check if a value has Dictionary type (flash.utils::Dictionary).
    fn is_dictionary(&self, v: ValueId) -> bool {
        match self.func.value_types.get(v) {
            Some(Type::Instance(id)) => {
                let name = self.type_name(*id);
                name.rsplit("::").next() == Some("Dictionary")
            }
            // AS3 Dictionary fields are often typed as Map<k, v> after type inference.
            // Map<K,V> doesn't have an index signature in TypeScript, so bracket access
            // produces TS7052. Treat any Map-typed value as a dictionary to emit .get()/.set().
            Some(Type::Map(_, _)) => true,
            _ => false,
        }
    }

    /// Check if a Instance-typed collection needs `(as any)` wrapping for bracket access.
    ///
    /// Returns true when the collection is a concrete class (Instance) that isn't
    /// Object/Class/Dictionary — types that already have index signatures in TS.
    fn is_struct_needing_index_coerce(&self, v: ValueId) -> bool {
        if let Some(Type::Instance(id)) = self.func.value_types.get(v) {
            let name = self.type_name(*id);
            let short = name.rsplit("::").next().unwrap_or(name);
            !matches!(short, "Object" | "Class" | "Dictionary")
        } else {
            false
        }
    }

    /// Check if a value is XML or XMLList typed (needs String coercion when used as index).
    fn is_xml_typed(&self, v: ValueId) -> bool {
        if let Some(Type::Instance(id)) = self.func.value_types.get(v) {
            let name = self.type_name(*id);
            let short = name.rsplit("::").next().unwrap_or(name);
            matches!(short, "XML" | "XMLList")
        } else {
            false
        }
    }

    /// Build an expression for a value reference.
    fn build_val(&mut self, v: ValueId) -> Expr {
        // Constants — always inlined, not consumed.
        if let Some(c) = self.resolve.constant_inlines.get(&v) {
            return Expr::Literal(c.clone());
        }

        // Always-inline — rebuilt on every use.
        if let Some(&inst_id) = self.always_inline_map.get(&v) {
            let op = self.func.insts[inst_id].op.clone();
            if let Some(expr) = self.build_expr_from_op(&op) {
                if self.config.wrap_class_refs_as_any {
                    if let Type::ClassRef(_) = &self.func.value_types[v] {
                        return Expr::Cast {
                            expr: Box::new(expr),
                            ty: self.func.value_types[v].clone(),
                            kind: CastKind::NullableCoerce,
                        };
                    }
                }
                return expr;
            }
        }

        // Side-effecting inline — consumed once.
        if let Some(expr) = self.side_effecting_inlines.remove(&v) {
            return expr;
        }

        // Lazy inline — consumed once.
        if let Some(inst_id) = self.pending_lazy.remove(&v) {
            let op = self.func.insts[inst_id].op.clone();
            if let Some(expr) = self.build_expr_from_op(&op) {
                if self.config.wrap_class_refs_as_any {
                    if let Type::ClassRef(_) = &self.func.value_types[v] {
                        return Expr::Cast {
                            expr: Box::new(expr),
                            ty: self.func.value_types[v].clone(),
                            kind: CastKind::NullableCoerce,
                        };
                    }
                }
                return expr;
            }
        }

        // Track block-param references for declaration generation.
        // Skip values that already have their own declaration emitted:
        // - se_flush_declared: emitted by flush_side_effecting_inlines
        // - or_inline_declared: emitted by emit_or_inline (count >= 2)
        if !self.entry_params.contains(&v)
            && !self.se_flush_declared.contains(&v)
            && !self.or_inline_declared.contains(&v)
        {
            self.referenced_block_params.insert(v);
        }

        Expr::Var(self.value_name(v))
    }

    /// Build an Expr from an Op.
    fn build_expr_from_op(&mut self, op: &Op) -> Option<Expr> {
        Some(match op {
            Op::Const(c) => Expr::Literal(c.clone()),

            Op::Load(ptr) => self.build_val(*ptr),
            Op::GetField { object, field } => Expr::Field {
                object: Box::new(self.build_val(*object)),
                field: field.clone(),
            },
            Op::GetIndex { collection, index } => {
                if self.is_dictionary(*collection) {
                    // Dictionary -> Map: dict.get(key)
                    Expr::CallIndirect {
                        callee: Box::new(Expr::Field {
                            object: Box::new(self.build_val(*collection)),
                            field: "get".into(),
                        }),
                        args: vec![self.build_val(*index)],
                    }
                } else if let Some(Constant::String(s)) = self.resolve.constant_inlines.get(index) {
                    if is_js_ident(s) {
                        let field = s.clone();
                        Expr::Field {
                            object: Box::new(self.build_val(*collection)),
                            field,
                        }
                    } else {
                        Expr::Index {
                            collection: Box::new(self.build_val(*collection)),
                            index: Box::new(self.build_val(*index)),
                        }
                    }
                } else {
                    let mut coll_expr = self.build_val(*collection);
                    let mut idx_expr = self.build_val(*index);
                    // Numeric-typed (Int/UInt/Float) collection: TypeScript's
                    // `Number` type has no index signature → TS7053. Cast to
                    // `any` so the index access compiles. GML uses integer
                    // object IDs interchangeably with arrays in some patterns.
                    if matches!(
                        self.func.value_types.get(*collection),
                        Some(Type::Int(_) | Type::UInt(_) | Type::Float(_))
                    ) {
                        coll_expr = Expr::Cast {
                            expr: Box::new(coll_expr),
                            ty: Type::Value,
                            kind: CastKind::NullableCoerce,
                        };
                    }
                    if self.config.coerce_index_types {
                        // Struct-typed collection + Unknown index → (collection as any)
                        if self.is_struct_needing_index_coerce(*collection)
                            && matches!(self.func.value_types.get(*index), Some(Type::Value) | None)
                        {
                            coll_expr = Expr::Cast {
                                expr: Box::new(coll_expr),
                                ty: Type::Value,
                                kind: CastKind::NullableCoerce,
                            };
                        }
                        // XML/XMLList index → String(index)
                        if self.is_xml_typed(*index) {
                            idx_expr = Expr::Cast {
                                expr: Box::new(idx_expr),
                                ty: Type::String,
                                kind: CastKind::Coerce,
                            };
                        }
                    }
                    Expr::Index {
                        collection: Box::new(coll_expr),
                        index: Box::new(idx_expr),
                    }
                }
            }

            Op::Call {
                func: callee_fid,
                args,
            } => {
                // Resolve the FuncId to its canonical name via the config's func_names map.
                // Falls back to a debug string if the name is not in the map (runtime-only
                // function not registered in the module's runtime_registry).
                let fname = self
                    .config
                    .func_names
                    .get(callee_fid)
                    .map(|s| s.as_str())
                    .unwrap_or("<unknown>");
                // Typed arithmetic / logic builtins are emitted as native
                // target-language operators rather than function calls.
                // Recognised by membership in `pure_fids` (the set of core
                // builtin FuncIds registered by `register_core_builtins`).
                if self.config.pure_fids.contains(callee_fid) {
                    self.build_builtin_expr(fname, args)
                } else {
                    Expr::Call {
                        func: fname.to_string(),
                        args: args.iter().map(|a| self.build_val(*a)).collect(),
                    }
                }
            }
            Op::CallIndirect { callee, args } => {
                let callee_ty = self.func.value_types[*callee].clone();
                let callee_expr = self.build_val(*callee);
                // When `cast_unknown_indirect_callee` is enabled and the callee is
                // `unknown` (Type::Value), calling it directly causes TS2571.
                // Cast to a typed function with matching arity so the call is valid.
                // Params and return type remain `unknown`; downstream uses of the
                // return value that need a concrete type require their own cast.
                //
                // Not enabled for Flash/GML: those backends have scope-resolution
                // rewrites (e.g. findPropStrict → bare name) that run after core emit
                // and cannot see through a cast wrapper.
                let callee_expr =
                    if self.config.cast_unknown_indirect_callee && callee_ty == Type::Value {
                        Expr::Cast {
                            expr: Box::new(callee_expr),
                            ty: Type::Function(Box::new(FunctionSig {
                                params: vec![Type::Value; args.len()],
                                return_ty: Type::Value,
                                ..Default::default()
                            })),
                            kind: CastKind::NullableCoerce,
                        }
                    } else {
                        callee_expr
                    };
                Expr::CallIndirect {
                    callee: Box::new(callee_expr),
                    args: args.iter().map(|a| self.build_val(*a)).collect(),
                }
            }
            Op::SystemCall {
                system,
                method,
                args,
            } => {
                // Cast Unknown constructor callee for SugarCube.Engine.new so that
                // `new (Engine.resolve("DateTime"))()` doesn't produce TS2571.
                // Same flag as cast_unknown_indirect_callee — both are Engine.resolve
                // patterns that need a cast when the callee is untyped.
                if system == "SugarCube.Engine"
                    && method == "new"
                    && self.config.cast_unknown_indirect_callee
                    && !args.is_empty()
                    && self.func.value_types.get(args[0]) == Some(&Type::Value)
                {
                    let callee_cast = Expr::Cast {
                        expr: Box::new(self.build_val(args[0])),
                        ty: Type::Value,
                        kind: CastKind::NullableCoerce,
                    };
                    let rest: Vec<_> = args[1..].iter().map(|a| self.build_val(*a)).collect();
                    return Some(Expr::SystemCall {
                        system: system.clone(),
                        method: method.clone(),
                        args: std::iter::once(callee_cast).chain(rest).collect(),
                    });
                }

                // Dictionary-specific rewrites for Flash.Object operations.
                if system == "Flash.Object" && args.len() >= 2 && self.is_dictionary(args[0]) {
                    match method.as_str() {
                        // deleteProperty(dict, key) -> dict.delete(key)
                        "deleteProperty" => {
                            return Some(Expr::CallIndirect {
                                callee: Box::new(Expr::Field {
                                    object: Box::new(self.build_val(args[0])),
                                    field: "delete".into(),
                                }),
                                args: vec![self.build_val(args[1])],
                            });
                        }
                        // hasProperty(dict, key) -> dict.has(key)
                        "hasProperty" => {
                            return Some(Expr::CallIndirect {
                                callee: Box::new(Expr::Field {
                                    object: Box::new(self.build_val(args[0])),
                                    field: "has".into(),
                                }),
                                args: vec![self.build_val(args[1])],
                            });
                        }
                        _ => {}
                    }
                }
                Expr::SystemCall {
                    system: system.clone(),
                    method: method.clone(),
                    args: args.iter().map(|a| self.build_val(*a)).collect(),
                }
            }

            Op::MethodCall {
                receiver,
                method,
                args,
            } => Expr::MethodCall {
                receiver: Box::new(self.build_val(*receiver)),
                method: method.clone(),
                args: args.iter().map(|a| self.build_val(*a)).collect(),
            },

            Op::Cast(v, ty, kind) => {
                if self
                    .func
                    .value_types
                    .get(*v)
                    .map(|t| t == ty)
                    .unwrap_or(false)
                {
                    self.build_val(*v)
                } else {
                    Expr::Cast {
                        expr: Box::new(self.build_val(*v)),
                        ty: ty.clone(),
                        kind: *kind,
                    }
                }
            }
            Op::TypeCheck(v, ty) => Expr::TypeCheck {
                expr: Box::new(self.build_val(*v)),
                ty: ty.clone(),
            },

            Op::StructInit { name, fields } => Expr::StructInit {
                name: name.clone(),
                fields: fields
                    .iter()
                    .map(|(n, v)| (n.clone(), self.build_val(*v)))
                    .collect(),
            },
            Op::ArrayInit(elems) => {
                Expr::ArrayInit(elems.iter().map(|v| self.build_val(*v)).collect())
            }
            Op::TupleInit(elems) => {
                Expr::TupleInit(elems.iter().map(|v| self.build_val(*v)).collect())
            }

            Op::Yield(v) => Expr::Yield(v.map(|yv| Box::new(self.build_val(yv)))),
            Op::CoroutineCreate { func: fname, args } => Expr::CoroutineCreate {
                func: fname.clone(),
                args: args.iter().map(|a| self.build_val(*a)).collect(),
            },
            Op::CoroutineResume(v) => Expr::CoroutineResume(Box::new(self.build_val(*v))),

            Op::MakeClosure { func, captures } => Expr::MakeClosure {
                func: func.clone(),
                captures: captures.iter().map(|&c| self.build_val(c)).collect(),
            },

            Op::GlobalRef(name) => Expr::GlobalRef(name.clone()),
            Op::Spread(v) => Expr::Spread(Box::new(self.build_val(*v))),
            Op::Alloc(_) | Op::Store { .. } | Op::SetField { .. } | Op::SetIndex { .. } => {
                return None
            }
        })
    }

    /// Map a core builtin name to a target-language expression.
    ///
    /// The `op_name` argument is the full builtin name (e.g. `"add_f64"`
    /// or `"not_bool"`).  Every match arm uses the full op name including the
    /// type suffix — no suffix stripping.  Binary ops take exactly 2 args;
    /// unary ops take 1.
    ///
    /// For bitwise ops on boolean operands the backend applies the same TS2447
    /// workaround (cast to Number) that the old Op::BitAnd/BitOr/BitXor arms
    /// applied before the IR variants were removed.
    fn build_builtin_expr(&mut self, op_name: &str, args: &[ValueId]) -> Expr {
        // Short-circuit boolean ops and select are control flow, not arithmetic — emit
        // as dedicated AST variants rather than opaque builtin calls.
        match op_name {
            "and_bool" => {
                return Expr::LogicalAnd {
                    lhs: Box::new(self.build_val(args[0])),
                    rhs: Box::new(self.build_val(args[1])),
                };
            }
            "or_bool" => {
                return Expr::LogicalOr {
                    lhs: Box::new(self.build_val(args[0])),
                    rhs: Box::new(self.build_val(args[1])),
                };
            }
            "select" => {
                return Expr::Ternary {
                    cond: Box::new(self.build_val(args[0])),
                    then_val: Box::new(self.build_val(args[1])),
                    else_val: Box::new(self.build_val(args[2])),
                };
            }
            _ => {}
        }

        // For bitwise ops, check if either operand is Bool and encode that
        // in the op name so the backend can apply the TS2447 workaround
        // without needing IR access.
        // TODO(Law 2): bitand_bool_i32 re-encoding is TS2447-specific — move to TS backend.
        let effective_name = match op_name {
            "bitand_i32" | "bitor_i32" | "bitxor_i32"
                if matches!(self.func.value_types[args[0]], Type::Bool)
                    || matches!(self.func.value_types[args[1]], Type::Bool) =>
            {
                match op_name {
                    "bitand_i32" => "bitand_bool_i32",
                    "bitor_i32" => "bitor_bool_i32",
                    "bitxor_i32" => "bitxor_bool_i32",
                    _ => unreachable!(),
                }
            }
            _ => op_name,
        };

        Expr::Call {
            func: effective_name.to_string(),
            args: args.iter().map(|&a| self.build_val(a)).collect(),
        }
    }

    /// Build a condition expression, wrapping void-typed values in a dynamic
    /// cast.  TypeScript disallows void expressions as boolean conditions
    /// (TS1345), so `void_call()` must become `(void_call() as any)`.
    fn build_bool_cond(&mut self, cond: ValueId) -> Expr {
        let expr = self.build_val(cond);
        if matches!(self.func.value_types[cond], Type::Void) {
            Expr::Cast {
                expr: Box::new(expr),
                ty: Type::Value,
                kind: CastKind::NullableCoerce,
            }
        } else {
            expr
        }
    }

    fn negate_cond(&mut self, cond: ValueId) -> Expr {
        if let Some(&inst_id) = self.pending_lazy.get(&cond) {
            // `cmp_*(a, b)` — flip to the inverse comparison builtin.
            if let Op::Call { func: fid, args } = &self.func.insts[inst_id].op {
                let fname = self
                    .config
                    .func_names
                    .get(fid)
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let inv = match fname {
                    "cmp_eq" => Some("cmp_ne"),
                    "cmp_ne" => Some("cmp_eq"),
                    "cmp_lt" => Some("cmp_ge"),
                    "cmp_ge" => Some("cmp_lt"),
                    "cmp_gt" => Some("cmp_le"),
                    "cmp_le" => Some("cmp_gt"),
                    _ => None,
                };
                if let (Some(inv_name), [a, b]) = (inv, args.as_slice()) {
                    let a = *a;
                    let b = *b;
                    self.pending_lazy.remove(&cond);
                    return Expr::Call {
                        func: inv_name.to_string(),
                        args: vec![self.build_val(a), self.build_val(b)],
                    };
                }
            }
            // `not_bool(x)` is boolean NOT — unwrap the inner value
            // instead of double-negating.
            if let Op::Call { func: fid, args } = &self.func.insts[inst_id].op {
                let fname = self
                    .config
                    .func_names
                    .get(fid)
                    .map(|s| s.as_str())
                    .unwrap_or("");
                if fname == "not_bool" && args.len() == 1 {
                    let inner = args[0];
                    self.pending_lazy.remove(&cond);
                    return self.build_val(inner);
                }
            }
        }
        Expr::Not(Box::new(self.build_val(cond)))
    }

    /// Flush all pending side-effecting inline expressions as statements.
    fn flush_side_effecting_inlines(&mut self, stmts: &mut Vec<Stmt>) {
        let mut to_flush: Vec<ValueId> = self.side_effecting_inlines.keys().copied().collect();
        to_flush.sort_by_key(|v| v.index());
        for v in to_flush {
            let expr = self.side_effecting_inlines.remove(&v).unwrap();
            self.se_flush_declared.insert(v);
            let name = self.value_name(v);
            if self.all_block_params.contains(&v) {
                // True block param: ensure collect_block_param_decls emits
                // `let name;` by inserting into referenced_block_params now,
                // then emit Assign (not VarDecl) to avoid a duplicate decl if
                // the phi is also assigned in a branch that runs later (which
                // would also insert into referenced_block_params there).
                self.referenced_block_params.insert(v);
                stmts.push(Stmt::Assign {
                    target: Expr::Var(name),
                    value: expr,
                });
            } else if self.shared_names.contains(&name) {
                // Shared name: a `let name;` will come from collect_block_param_decls
                // (via the shared_names loop). Emit Assign only.
                stmts.push(Stmt::Assign {
                    target: Expr::Var(name),
                    value: expr,
                });
            } else {
                // Non-block-param with no pre-existing declaration (e.g. a
                // side-effecting value that fell through build_val during an
                // emit_logical_and/or rhs_body). Emit a self-contained VarDecl.
                stmts.push(Stmt::VarDecl {
                    name,
                    ty: None,
                    init: Some(expr),
                    mutable: false,
                });
            }
        }
    }

    /// Flush deferred memory-read lazy inlines into real statements.
    fn flush_pending_reads(&mut self, stmts: &mut Vec<Stmt>) {
        let mut to_flush: Vec<(ValueId, InstId)> = self
            .pending_lazy
            .iter()
            .filter(|(&v, &iid)| {
                !self.flush_protected.contains(&v)
                    && matches!(
                        self.func.insts[iid].op,
                        Op::GetField { .. } | Op::GetIndex { .. } | Op::Load(..)
                    )
            })
            .map(|(&v, &iid)| (v, iid))
            .collect();
        // Sort by ValueId for deterministic flush order.
        to_flush.sort_by_key(|(v, _)| v.index());

        // Build map: unnamed flush value -> named Cast/Copy consumer in pending_lazy.
        // When a GetField is flushed but its only consumer is a named Cast (e.g.
        // Mem2Reg named the Cast from an alloc's stored value), absorb the Cast
        // into the flush so the materialized variable gets the source-level name.
        let named_consumers: HashMap<ValueId, (ValueId, InstId)> = self
            .pending_lazy
            .iter()
            .filter_map(|(&w, &wiid)| {
                if !self.value_names.contains_key(&w) {
                    return None;
                }
                match &self.func.insts[wiid].op {
                    Op::Cast(src, ..) => Some((*src, (w, wiid))),
                    _ => None,
                }
            })
            .collect();

        for (v, iid) in to_flush {
            if !self.pending_lazy.contains_key(&v) {
                continue;
            }

            // If this value has no name but has a named Cast/Copy consumer in
            // pending_lazy, materialize the consumer instead.  build_expr_from_op
            // on the Cast will call build_val(v) which consumes v from pending_lazy.
            if !self.value_names.contains_key(&v) {
                if let Some(&(consumer_v, _)) = named_consumers.get(&v) {
                    if let Some(consumer_iid) = self.pending_lazy.remove(&consumer_v) {
                        let op = self.func.insts[consumer_iid].op.clone();
                        if let Some(expr) = self.build_expr_from_op(&op) {
                            let name = self.value_name(consumer_v);
                            if self.shared_names.contains(&name) {
                                stmts.push(Stmt::Assign {
                                    target: Expr::Var(name),
                                    value: expr,
                                });
                            } else {
                                stmts.push(Stmt::VarDecl {
                                    name,
                                    ty: None,
                                    init: Some(expr),
                                    mutable: false,
                                });
                            }
                            continue;
                        }
                    }
                }
            }

            // Normal flush: remove from pending and materialize.
            if self.pending_lazy.remove(&v).is_none() {
                continue;
            }
            let op = self.func.insts[iid].op.clone();
            if let Some(expr) = self.build_expr_from_op(&op) {
                let name = self.value_name(v);
                if self.shared_names.contains(&name) {
                    stmts.push(Stmt::Assign {
                        target: Expr::Var(name),
                        value: expr,
                    });
                } else {
                    stmts.push(Stmt::VarDecl {
                        name,
                        ty: None,
                        init: Some(expr),
                        mutable: false,
                    });
                }
            }
        }
    }

    /// Either inline a single-use expression or emit it as a statement.
    fn emit_or_inline(&mut self, v: ValueId, expr: Expr, stmts: &mut Vec<Stmt>) {
        let count = self.use_count(v);
        if count == 1 {
            self.side_effecting_inlines.insert(v, expr);
        } else if count == 0 {
            stmts.push(Stmt::Expr(expr));
        } else {
            // count >= 2: materialize into a named variable.
            let name = self.value_name(v);
            if self.all_block_params.contains(&v) {
                // Actual block params used across scope boundaries: hoist a `let`
                // declaration to function scope and emit an Assign here.
                // Do NOT add to or_inline_declared — build_val must still be
                // able to insert into referenced_block_params on later uses.
                self.referenced_block_params.insert(v);
                stmts.push(Stmt::Assign {
                    target: Expr::Var(name),
                    value: expr,
                });
            } else {
                self.or_inline_declared.insert(v);
                if self.shared_names.contains(&name) {
                    // A hoisted `let name;` exists — emit Assign only.
                    stmts.push(Stmt::Assign {
                        target: Expr::Var(name),
                        value: expr,
                    });
                } else {
                    stmts.push(Stmt::VarDecl {
                        name,
                        ty: None,
                        init: Some(expr),
                        mutable: false,
                    });
                }
            }
        }
    }

    /// Collect block-param declarations for non-entry blocks, plus any
    /// shared names that lack a block-param declaration (from Cast/Copy
    /// name propagation).
    pub(super) fn collect_block_param_decls(&self) -> Vec<Stmt> {
        let mut decls = Vec::new();
        let mut declared = HashSet::new();
        for p in &self.func.blocks[self.func.entry].params {
            declared.insert(self.value_name(p.value));
        }
        for (block_id, block) in self.func.blocks.iter() {
            if block_id == self.func.entry {
                continue;
            }
            for param in &block.params {
                let name = self.value_name(param.value);
                if self.referenced_block_params.contains(&param.value)
                    && declared.insert(name.clone())
                {
                    // Use coalesced_decl_types: if multiple block params share
                    // this name with different types, the map holds Unknown.
                    // Fall back to value_types for names with a single type.
                    let ty = self
                        .coalesced_decl_types
                        .get(&name)
                        .cloned()
                        .unwrap_or_else(|| self.func.value_types[param.value].clone());
                    decls.push(Stmt::VarDecl {
                        name,
                        ty: Some(ty),
                        init: None,
                        mutable: true,
                    });
                }
            }
        }
        // Shared names without a block-param declaration need an uninit let.
        // This happens when Cast/Copy name propagation creates duplicate names
        // for a Cast result and its source — both emit Assign, but neither
        // generates a VarDecl.
        // Sort for deterministic declaration order.
        let mut sorted_shared: Vec<_> = self.shared_names.iter().collect();
        sorted_shared.sort();
        for name in sorted_shared {
            if declared.insert(name.clone()) {
                // Use coalesced_decl_types if available — this holds the widened
                // type (Unknown) when multiple values share the name with different
                // types.  Fall back to cross_scope_hoisted_types for closure-lifted
                // variables, then None (no annotation) as last resort.
                let ty = self
                    .coalesced_decl_types
                    .get(name)
                    .cloned()
                    .or_else(|| self.cross_scope_hoisted_types.get(name).cloned());
                decls.push(Stmt::VarDecl {
                    name: name.clone(),
                    ty,
                    init: None,
                    mutable: true,
                });
            }
        }
        decls
    }

    pub(super) fn build_params(&self) -> Vec<(String, Type)> {
        let mut seen = HashSet::new();
        self.func.blocks[self.func.entry]
            .params
            .iter()
            .map(|p| {
                let mut name = self.value_name(p.value);
                if !seen.insert(name.clone()) {
                    // Duplicate parameter name — append a suffix.
                    let base = name.clone();
                    let mut i = 2;
                    loop {
                        name = format!("{base}{i}");
                        if seen.insert(name.clone()) {
                            break;
                        }
                        i += 1;
                    }
                }
                (name, p.ty.clone())
            })
            .collect()
    }

    // -------------------------------------------------------------------
    // Statement emission
    // -------------------------------------------------------------------

    pub(super) fn emit_stmts(&mut self, stmts: &[LinearStmt]) -> Vec<Stmt> {
        let mut out = Vec::new();
        self.emit_stmts_into(stmts, &mut out);
        out
    }

    fn emit_stmts_into(&mut self, stmts: &[LinearStmt], out: &mut Vec<Stmt>) {
        for stmt in stmts {
            self.emit_one(stmt, out);
        }
    }

    /// Emit statements in a nested scope, protecting current pending reads
    /// from being flushed by memory writes inside the body.
    fn emit_stmts_protected(&mut self, stmts: &[LinearStmt]) -> Vec<Stmt> {
        let newly_protected: Vec<ValueId> = self
            .pending_lazy
            .keys()
            .filter(|v| !self.flush_protected.contains(v))
            .copied()
            .collect();
        self.flush_protected.extend(newly_protected.iter().copied());
        let result = self.emit_stmts(stmts);
        for v in &newly_protected {
            self.flush_protected.remove(v);
        }
        result
    }

    fn emit_one(&mut self, stmt: &LinearStmt, stmts: &mut Vec<Stmt>) {
        match stmt {
            LinearStmt::Def { result, inst_id } => self.emit_def(*result, *inst_id, stmts),
            LinearStmt::Effect { inst_id } => self.emit_effect(*inst_id, stmts),
            LinearStmt::Assign { dst, src } => self.emit_assign(*dst, *src, stmts),
            LinearStmt::Return { value } => {
                stmts.push(Stmt::Return(value.map(|v| self.build_val(v))));
            }
            LinearStmt::Break => stmts.push(Stmt::Break),
            LinearStmt::Continue => stmts.push(Stmt::Continue),
            LinearStmt::LabeledBreak { depth } => {
                stmts.push(Stmt::LabeledBreak { depth: *depth });
            }
            LinearStmt::If {
                cond,
                then_body,
                else_body,
            } => self.emit_if(*cond, then_body, else_body, stmts),
            LinearStmt::While {
                header,
                cond,
                cond_negated,
                body,
            } => self.emit_while(header, *cond, *cond_negated, body, stmts),
            LinearStmt::For {
                init,
                header,
                cond,
                cond_negated,
                update,
                body,
            } => self.emit_for(init, header, *cond, *cond_negated, update, body, stmts),
            LinearStmt::Loop { body } => {
                let body_stmts = self.emit_stmts_protected(body);
                stmts.push(Stmt::Loop { body: body_stmts });
            }
            LinearStmt::LogicalOr {
                cond,
                phi,
                rhs_body,
                rhs,
            } => self.emit_logical_or(*cond, *phi, rhs_body, *rhs, stmts),
            LinearStmt::LogicalAnd {
                cond,
                phi,
                rhs_body,
                rhs,
            } => self.emit_logical_and(*cond, *phi, rhs_body, *rhs, stmts),
            LinearStmt::Dispatch { blocks, entry } => {
                let mut dispatch_blocks = Vec::new();
                for (id, block_stmts) in blocks {
                    let emitted = self.emit_stmts_protected(block_stmts);
                    dispatch_blocks.push((*id, emitted));
                }
                stmts.push(Stmt::Dispatch {
                    blocks: dispatch_blocks,
                    entry: *entry,
                });
            }
            LinearStmt::Switch {
                value,
                cases,
                default_body,
            } => {
                // Flush pending SE inlines before entering switch cases.
                // A SE inline flushed inside a case body is only in scope for
                // that case — uses in other cases would be undeclared (TS2304).
                // Same pattern as emit_if.
                self.flush_side_effecting_inlines(stmts);
                let val = self.build_val(*value);
                let mut case_stmts = Vec::new();
                for (constant, body) in cases {
                    let emitted = self.emit_stmts_protected(body);
                    case_stmts.push((constant.clone(), emitted));
                }
                let default_stmts = self.emit_stmts_protected(default_body);
                stmts.push(Stmt::Switch {
                    value: val,
                    cases: case_stmts,
                    default_body: default_stmts,
                });
            }
        }
    }

    /// Wraps a `SystemCall` `construct` op with `.toString()` when the IR result
    /// type is `String`.  In AS3, constructing `XML`/`XMLList` from a string
    /// implicitly coerces the result to a string.  The IR captures this as a
    /// `String` result type on the `SystemCall`, but the TypeScript backend rewrites
    /// `construct` to `new XML(...)` whose TS type is `XML`, not `string`.
    /// Inserting `.toString()` here restores the correct string type and avoids
    /// TS2345/TS2322 errors at call sites.
    ///
    /// Only fires when `LoweringConfig::construct_string_coerce` is `true`
    /// (set by the Flash backend).  Other engines never produce this pattern.
    fn construct_string_coerce(enabled: bool, result_ty: &Type, op: &Op, expr: Expr) -> Expr {
        if !enabled {
            return expr;
        }
        if !matches!(result_ty, Type::String) {
            return expr;
        }
        if let Op::SystemCall { method, .. } = op {
            if method == "construct" {
                return Expr::MethodCall {
                    receiver: Box::new(expr),
                    method: "toString".into(),
                    args: vec![],
                };
            }
        }
        expr
    }

    fn emit_def(&mut self, result: ValueId, inst_id: InstId, stmts: &mut Vec<Stmt>) {
        let op = &self.func.insts[inst_id].op;

        // Phase 2 classified — defer or skip.
        if self.resolve.constant_inlines.contains_key(&result) {
            return;
        }
        if self.resolve.always_inlines.contains(&result) {
            self.always_inline_map.insert(result, inst_id);
            return;
        }
        if self.resolve.lazy_inlines.contains(&result)
            && !self.resolve.cross_scope_defs.contains(&result)
        {
            // If any operand is a pending SE inline, eagerly build the
            // expression and store as SE inline. This chains pure wrappers
            // (e.g. Cast) into their SE operand (e.g. Call) so the flush
            // materializes the combined expression with the correct name.
            let has_se_operand = value_operands(op)
                .iter()
                .any(|v| self.side_effecting_inlines.contains_key(v));
            if has_se_operand {
                let op_clone = op.clone();
                if let Some(expr) = self.build_expr_from_op(&op_clone) {
                    self.side_effecting_inlines.insert(result, expr);
                    return;
                }
            }
            self.pending_lazy.insert(result, inst_id);
            return;
        }

        let count = self.use_count(result);

        // Dead pure.
        if count == 0 && is_deferrable(op, &self.config.pure_fids) {
            return;
        }

        // Alloc -> VarDecl.
        if let Op::Alloc(ty) = op {
            let alloc_ty = ty.clone();
            let alloc_init_v = self.resolve.alloc_inits.get(&result).copied();
            let init = alloc_init_v.map(|iv| self.build_val(iv));
            // Suppress the type annotation when the init value's IR type is
            // incompatible with the alloc type (e.g. DefaultDict stored into an
            // Optional(Float) alloc due to type inference picking the wrong path).
            // TypeScript will infer the correct type from the init expression.
            let ty_ann = if let Some(init_v) = alloc_init_v {
                let init_ty = &self.func.value_types[init_v];
                if types_coalesce_compatible(&alloc_ty, init_ty) {
                    Some(alloc_ty)
                } else {
                    None
                }
            } else {
                Some(alloc_ty)
            };
            stmts.push(Stmt::VarDecl {
                name: self.value_name(result),
                ty: ty_ann,
                init,
                mutable: true,
            });
            return;
        }

        // Build expression.
        let op_clone = op.clone();
        let expr = self
            .build_expr_from_op(&op_clone)
            .unwrap_or_else(|| Expr::Var(self.value_name(result)));
        let result_ty = self.func.value_types[result].clone();
        let expr = Self::construct_string_coerce(
            self.config.construct_string_coerce,
            &result_ty,
            &op_clone,
            expr,
        );
        // Inject `as <type>` cast when type inference has narrowed a SystemCall
        // result that the runtime still declares as `unknown` in TypeScript.
        // Controlled by `LoweringConfig::cast_narrowed_syscall_results_for`.
        //
        // Cast kind selection:
        // - Scalar (Float/Int/Bool/String): NullableCoerce → `expr as number/boolean/string`
        // - Struct/Enum: Coerce → `expr as TypeName` (TS assertion, no runtime asType())
        // - Array: NullableCoerce → `expr as T[]` (propagates through array indexing)
        // Other types (Unknown, Void, Union, etc.) are skipped — no cast.
        let syscall_cast_kind = match &result_ty {
            Type::Float(_) | Type::Int(_) | Type::UInt(_) | Type::Bool | Type::String => {
                Some(CastKind::NullableCoerce)
            }
            Type::Instance(_) => Some(CastKind::Coerce),
            Type::Array(_) | Type::Function(_) => Some(CastKind::NullableCoerce),
            _ => None,
        };
        let expr = {
            // Determine whether to inject `as <type>` cast for narrowed syscall results.
            // Checks cast_narrowed_syscall_results_for and cast_struct_syscall_results_for
            // against the (system, method) pair from:
            // - Op::SystemCall: directly available.
            // - Op::Call with dotted name (e.g. "GameMaker.Instance.getField"): split at
            //   the last dot to get (system, method). Also checks intrinsic_calls map
            //   for Flash/Twine Op::Call intrinsics.
            let in_narrowed_or_struct_only = match &op_clone {
                Op::SystemCall { system, method, .. } => {
                    let in_narrowed = self
                        .config
                        .cast_narrowed_syscall_results_for
                        .iter()
                        .any(|(s, m)| s == system && m == method);
                    let in_struct_only = matches!(&result_ty, Type::Instance(_))
                        && self
                            .config
                            .cast_struct_syscall_results_for
                            .iter()
                            .any(|(s, m)| s == system && m == method);
                    in_narrowed || in_struct_only
                }
                Op::Call {
                    func: callee_fid, ..
                } => {
                    let fname = self.config.func_names.get(callee_fid).map(|s| s.as_str());
                    if let Some(fname) = fname {
                        // Check explicit intrinsic_calls map first (Flash/Twine).
                        let sm_from_map = self
                            .config
                            .intrinsic_calls
                            .get(fname)
                            .map(|(s, m)| (s.as_str(), m.as_str()));
                        // Fall back to dotted-name split (GML plain Op::Call).
                        let sm = sm_from_map.or_else(|| {
                            fname
                                .rfind('.')
                                .map(|dot| (&fname[..dot], &fname[dot + 1..]))
                        });
                        if let Some((system, method)) = sm {
                            let in_narrowed = self
                                .config
                                .cast_narrowed_syscall_results_for
                                .iter()
                                .any(|(s, m)| s == system && m == method);
                            let in_struct_only = matches!(&result_ty, Type::Instance(_))
                                && self
                                    .config
                                    .cast_struct_syscall_results_for
                                    .iter()
                                    .any(|(s, m)| s == system && m == method);
                            in_narrowed || in_struct_only
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                _ => false,
            };
            if in_narrowed_or_struct_only {
                if let Some(cast_kind) = syscall_cast_kind {
                    Expr::Cast {
                        expr: Box::new(expr),
                        ty: result_ty.clone(),
                        kind: cast_kind,
                    }
                } else {
                    expr
                }
            } else {
                expr
            }
        };
        let side_effecting = is_side_effecting_op(&op_clone);

        if count == 0 && side_effecting {
            stmts.push(Stmt::Expr(expr));
        } else if count == 1 && side_effecting {
            if self.resolve.cross_scope_defs.contains(&result) {
                // Value defined in a nested scope (e.g. an if-branch body)
                // but used in an outer scope (e.g. a sibling while loop).
                // The SE-inline flush would declare it inside the branch,
                // putting the name out of scope at the use site.  Instead,
                // treat it as a shared name: emit an Assign here and let
                // collect_block_param_decls hoist `let name!` to function
                // scope.
                let name = self.value_name(result);
                self.shared_names.insert(name.clone());
                // Record the type so collect_block_param_decls can emit
                // `let name!: ty;` (with definite-assignment assertion) instead
                // of the untyped `let name;` that would trigger TS2454.
                // Struct types are suppressed here for the same reason as in
                // types_coalesce_compatible: a Struct annotation can prevent
                // TypeScript from inferring `any` from e.g. instance_create_depth
                // when called with `cls as any`, causing TS2362 on arithmetic.
                let ty = self.func.value_types[result].clone();
                if !matches!(ty, Type::Instance(_)) {
                    self.cross_scope_hoisted_types.insert(name.clone(), ty);
                }
                stmts.push(Stmt::Assign {
                    target: Expr::Var(name),
                    value: expr,
                });
            } else {
                self.side_effecting_inlines.insert(result, expr);
            }
        } else {
            let name = self.value_name(result);
            if self.shared_names.contains(&name) {
                stmts.push(Stmt::Assign {
                    target: Expr::Var(name),
                    value: expr,
                });
            } else if self.resolve.cross_scope_defs.contains(&result) {
                // Multi-use value defined in a nested scope (e.g. loop body)
                // but used in an outer scope (e.g. after loop break).
                // Hoist the declaration to function scope via shared_names.
                self.shared_names.insert(name.clone());
                let ty = self.func.value_types[result].clone();
                if !matches!(ty, Type::Instance(_)) {
                    self.cross_scope_hoisted_types.insert(name.clone(), ty);
                }
                stmts.push(Stmt::Assign {
                    target: Expr::Var(name),
                    value: expr,
                });
            } else {
                stmts.push(Stmt::VarDecl {
                    name,
                    ty: None,
                    init: Some(expr),
                    mutable: false,
                });
            }
        }
    }

    fn emit_effect(&mut self, inst_id: InstId, stmts: &mut Vec<Stmt>) {
        let op = &self.func.insts[inst_id].op;

        // Flush pending reads before memory writes.
        if is_memory_write(op) {
            self.flush_pending_reads(stmts);
        }

        // Skip merged stores.
        if self.resolve.skip_stores.contains(&inst_id) {
            return;
        }

        let op = self.func.insts[inst_id].op.clone();
        match &op {
            Op::Store { ptr, value } => {
                let target = Expr::Var(self.value_name(*ptr));
                let val = self.build_val(*value);
                stmts.push(Stmt::Assign { target, value: val });
            }
            Op::SetField {
                object,
                field,
                value,
            } => {
                let target = Expr::Field {
                    object: Box::new(self.build_val(*object)),
                    field: field.clone(),
                };
                stmts.push(Stmt::Assign {
                    target,
                    value: self.build_val(*value),
                });
            }
            Op::SetIndex {
                collection,
                index,
                value,
            } => {
                if self.is_dictionary(*collection) {
                    // Dictionary -> Map: dict.set(key, value)
                    let dict = self.build_val(*collection);
                    let key = self.build_val(*index);
                    let val = self.build_val(*value);
                    stmts.push(Stmt::Expr(Expr::CallIndirect {
                        callee: Box::new(Expr::Field {
                            object: Box::new(dict),
                            field: "set".into(),
                        }),
                        args: vec![key, val],
                    }));
                } else if let Some(Constant::String(s)) = self.resolve.constant_inlines.get(index) {
                    let target = if is_js_ident(s) {
                        let field = s.clone();
                        Expr::Field {
                            object: Box::new(self.build_val(*collection)),
                            field,
                        }
                    } else {
                        Expr::Index {
                            collection: Box::new(self.build_val(*collection)),
                            index: Box::new(self.build_val(*index)),
                        }
                    };
                    let val = self.build_val(*value);
                    stmts.push(Stmt::Assign { target, value: val });
                } else {
                    let target = Expr::Index {
                        collection: Box::new(self.build_val(*collection)),
                        index: Box::new(self.build_val(*index)),
                    };
                    let val = self.build_val(*value);
                    stmts.push(Stmt::Assign { target, value: val });
                }
            }
            _ => {
                if let Some(expr) = self.build_expr_from_op(&op) {
                    stmts.push(Stmt::Expr(expr));
                }
            }
        }
    }

    fn emit_assign(&mut self, dst: ValueId, src: ValueId, stmts: &mut Vec<Stmt>) {
        // Skip identity assignments (same ValueId) — these are always no-ops.
        if dst == src {
            return;
        }
        let target_name = self.value_name(dst);
        let value = self.build_val(src);
        // Name-based self-assignments are cleaned up by eliminate_self_assigns
        // which runs before ternary rewrite.
        self.referenced_block_params.insert(dst);
        stmts.push(Stmt::Assign {
            target: Expr::Var(target_name),
            value,
        });
    }

    fn emit_if(
        &mut self,
        cond: ValueId,
        then_body: &[LinearStmt],
        else_body: &[LinearStmt],
        stmts: &mut Vec<Stmt>,
    ) {
        self.flush_side_effecting_inlines(stmts);

        // Emit bodies first so we know whether to negate the condition.
        // Use emit_stmts_protected so header-level pending reads aren't
        // flushed by memory writes inside the bodies.
        let then_stmts = self.emit_stmts_protected(then_body);
        let else_stmts = self.emit_stmts_protected(else_body);

        let then_empty = then_stmts.is_empty();
        let else_empty = else_stmts.is_empty();

        match (then_empty, else_empty) {
            (true, true) => {
                // Consume the condition so it doesn't become a dangling lazy.
                let _ = self.build_val(cond);
            }
            (false, true) => {
                stmts.push(Stmt::If {
                    cond: self.build_bool_cond(cond),
                    then_body: then_stmts,
                    else_body: Vec::new(),
                });
            }
            (true, false) => {
                // Negate condition and use else as then — can invert CmpKind.
                // For void conditions, negate_cond would emit !void which also
                // triggers TS1345; use build_bool_cond + Not instead.
                let negated = if matches!(self.func.value_types[cond], Type::Void) {
                    Expr::Not(Box::new(self.build_bool_cond(cond)))
                } else {
                    self.negate_cond(cond)
                };
                stmts.push(Stmt::If {
                    cond: negated,
                    then_body: else_stmts,
                    else_body: Vec::new(),
                });
            }
            (false, false) => {
                stmts.push(Stmt::If {
                    cond: self.build_bool_cond(cond),
                    then_body: then_stmts,
                    else_body: else_stmts,
                });
            }
        }
    }

    fn emit_while(
        &mut self,
        header: &[LinearStmt],
        cond: ValueId,
        cond_negated: bool,
        body: &[LinearStmt],
        stmts: &mut Vec<Stmt>,
    ) {
        // Flush any outer SE inlines before the loop. If we don't, nested
        // emit_if calls inside the loop body will flush them into the body's
        // stmts — creating declarations that are out of scope after the loop.
        self.flush_side_effecting_inlines(stmts);
        let mut header_stmts = Vec::new();
        self.emit_stmts_into(header, &mut header_stmts);

        if self.config.while_condition_hoisting && header_stmts.is_empty() {
            let cond_expr = if cond_negated {
                self.negate_cond(cond)
            } else {
                self.build_val(cond)
            };
            let mut body_stmts = self.emit_stmts_protected(body);
            strip_trailing_continue(&mut body_stmts);
            stmts.push(Stmt::While {
                cond: cond_expr,
                body: body_stmts,
            });
        } else {
            let break_expr = if cond_negated {
                self.build_val(cond)
            } else {
                self.negate_cond(cond)
            };
            header_stmts.push(Stmt::If {
                cond: break_expr,
                then_body: vec![Stmt::Break],
                else_body: Vec::new(),
            });
            let mut body_stmts = self.emit_stmts_protected(body);
            strip_trailing_continue(&mut body_stmts);
            header_stmts.append(&mut body_stmts);
            stmts.push(Stmt::Loop { body: header_stmts });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_for(
        &mut self,
        init: &[LinearStmt],
        header: &[LinearStmt],
        cond: ValueId,
        cond_negated: bool,
        update: &[LinearStmt],
        body: &[LinearStmt],
        stmts: &mut Vec<Stmt>,
    ) {
        // Flush any outer SE inlines before the loop (same reasoning as emit_while).
        self.flush_side_effecting_inlines(stmts);
        // Emit init assigns.
        self.emit_stmts_into(init, stmts);

        let mut header_stmts = Vec::new();
        self.emit_stmts_into(header, &mut header_stmts);

        if self.config.while_condition_hoisting && header_stmts.is_empty() {
            let cond_expr = if cond_negated {
                self.negate_cond(cond)
            } else {
                self.build_val(cond)
            };
            let mut body_stmts = self.emit_stmts_protected(body);
            strip_trailing_continue(&mut body_stmts);
            self.emit_stmts_into(update, &mut body_stmts);
            stmts.push(Stmt::While {
                cond: cond_expr,
                body: body_stmts,
            });
        } else {
            let break_expr = if cond_negated {
                self.build_val(cond)
            } else {
                self.negate_cond(cond)
            };
            header_stmts.push(Stmt::If {
                cond: break_expr,
                then_body: vec![Stmt::Break],
                else_body: Vec::new(),
            });
            let mut body_stmts = self.emit_stmts_protected(body);
            strip_trailing_continue(&mut body_stmts);
            header_stmts.append(&mut body_stmts);
            self.emit_stmts_into(update, &mut header_stmts);
            stmts.push(Stmt::Loop { body: header_stmts });
        }
    }

    fn emit_logical_or(
        &mut self,
        cond: ValueId,
        phi: ValueId,
        rhs_body: &[LinearStmt],
        rhs: ValueId,
        stmts: &mut Vec<Stmt>,
    ) {
        // Save SE inlines from header to prevent leaking into rhs_body.
        let saved_se = std::mem::take(&mut self.side_effecting_inlines);
        let body_stmts = self.emit_stmts_protected(rhs_body);
        let rhs_se = std::mem::replace(&mut self.side_effecting_inlines, saved_se);
        self.side_effecting_inlines.extend(rhs_se);

        if self.config.logical_operators && body_stmts.is_empty() {
            let expr = Expr::LogicalOr {
                lhs: Box::new(self.build_val(cond)),
                rhs: Box::new(self.build_val(rhs)),
            };
            self.emit_or_inline(phi, expr, stmts);
        } else {
            let cond_expr = self.build_val(cond);
            let then_stmts = vec![Stmt::Assign {
                target: Expr::Var(self.value_name(phi)),
                // Reuse the already-built cond expression — build_val(cond)
                // would fail here because the lazy inline was consumed above.
                value: cond_expr.clone(),
            }];
            let mut else_stmts = body_stmts;
            if rhs != phi {
                else_stmts.push(Stmt::Assign {
                    target: Expr::Var(self.value_name(phi)),
                    value: self.build_val(rhs),
                });
            }
            self.referenced_block_params.insert(phi);
            stmts.push(Stmt::If {
                cond: cond_expr,
                then_body: then_stmts,
                else_body: else_stmts,
            });
        }
    }

    fn emit_logical_and(
        &mut self,
        cond: ValueId,
        phi: ValueId,
        rhs_body: &[LinearStmt],
        rhs: ValueId,
        stmts: &mut Vec<Stmt>,
    ) {
        let saved_se = std::mem::take(&mut self.side_effecting_inlines);
        let body_stmts = self.emit_stmts_protected(rhs_body);
        let rhs_se = std::mem::replace(&mut self.side_effecting_inlines, saved_se);
        self.side_effecting_inlines.extend(rhs_se);

        if self.config.logical_operators && body_stmts.is_empty() {
            let expr = Expr::LogicalAnd {
                lhs: Box::new(self.build_val(cond)),
                rhs: Box::new(self.build_val(rhs)),
            };
            self.emit_or_inline(phi, expr, stmts);
        } else {
            let cond_expr = self.build_val(cond);
            let mut then_stmts = body_stmts;
            if rhs != phi {
                then_stmts.push(Stmt::Assign {
                    target: Expr::Var(self.value_name(phi)),
                    value: self.build_val(rhs),
                });
            }
            let else_stmts = vec![Stmt::Assign {
                target: Expr::Var(self.value_name(phi)),
                // Reuse the already-built cond expression — build_val(cond)
                // would fail here because the lazy inline was consumed above.
                value: cond_expr.clone(),
            }];
            self.referenced_block_params.insert(phi);
            stmts.push(Stmt::If {
                cond: cond_expr,
                then_body: then_stmts,
                else_body: else_stmts,
            });
        }
    }
}
