/// Configuration for debug dumps during the pipeline.
///
/// When enabled, dumps IR and/or AST to stderr at key points. An optional
/// function filter restricts output to matching functions (see
/// [`DebugConfig::should_dump`] for matching rules).
#[derive(Debug, Clone, Default)]
pub struct DebugConfig {
    /// Dump post-transform IR to stderr before structurization.
    pub dump_ir: bool,
    /// Dump raw AST to stderr before AST-to-AST passes.
    pub dump_ast: bool,
    /// Filter dumps to functions whose name matches this string.
    ///
    /// Matching is flexible: plain substring, case-insensitive substring, and
    /// split-part matching on `.`/`::` separators are all tried. See
    /// [`DebugConfig::should_dump`] for the full rules.
    pub function_filter: Option<String>,
    /// Stop the transform pipeline after the named pass, dump IR, then exit
    /// without emitting code.
    ///
    /// Pass names use the same kebab-case as `--skip-pass` (e.g.
    /// `"type-inference"`, `"mem2reg"`). The special value `"frontend"` dumps
    /// raw IR before any transforms run.  Honoured by
    /// [`crate::pipeline::TransformPipeline::run_with_debug`].
    pub dump_ir_after: Option<String>,
    /// Print per-pass wall-clock timing to stderr after each pass completes.
    ///
    /// Output format: `[timing] pass-name: Xms` (non-fixpoint) or
    /// `[timing] iter N: pass-name: Xms` (fixpoint).
    pub timing: bool,
}

impl DebugConfig {
    /// A config with all dumps disabled.
    pub fn none() -> Self {
        Self::default()
    }

    /// Returns `true` if no filter is set, or if the function name matches
    /// the filter under any of these strategies (tried in order):
    ///
    /// 1. **Case-sensitive substring** — `"step"` matches `"Gun::event_step_2"`.
    /// 2. **Case-insensitive substring** — `"STEP"` matches `"Gun::event_step_2"`.
    /// 3. **Split-part matching** — if the filter contains `.` or `::`, split on
    ///    those separators and require all parts to appear in the name as
    ///    case-insensitive substrings. This lets users write `"Gun.step"` or
    ///    `"Gun::step"` and match `"Gun::event_step_2"`.
    ///
    /// IR function names use `::` as the class/method separator (e.g.
    /// `"Gun::event_step_2"`, `"ClassName::methodName"`). Free functions have
    /// no separator (e.g. `"scr_init"`).
    pub fn should_dump(&self, func_name: &str) -> bool {
        let Some(filter) = self.function_filter.as_deref() else {
            return true;
        };

        // Strategy 1: case-sensitive substring (original behaviour).
        if func_name.contains(filter) {
            return true;
        }

        // Strategy 2: case-insensitive substring.
        let name_lower = func_name.to_lowercase();
        let filter_lower = filter.to_lowercase();
        if name_lower.contains(&filter_lower) {
            return true;
        }

        // Strategy 3: split-part matching — split filter on `.` and `::`,
        // require every non-empty part to appear in the lowercased name.
        if filter.contains('.') || filter.contains("::") {
            let parts: Vec<&str> = filter.split(['.', ':']).filter(|p| !p.is_empty()).collect();
            if !parts.is_empty() && parts.iter().all(|p| name_lower.contains(&p.to_lowercase())) {
                return true;
            }
        }

        false
    }
}

/// Configuration for which transform passes to run.
///
/// All passes are enabled by default. Disable individual passes by setting
/// their fields to `false`, or use `from_skip_list` with pass name strings.
#[derive(Debug, Clone)]
pub struct PassConfig {
    /// Infer struct definitions from constructor `SetField` ops.
    /// Runs before `ConstraintSolveHM` so field types are available to the solver.
    pub constructor_struct_infer: bool,
    /// HM-style constraint solver. Replaces the old `TypeInference`,
    /// `CallSiteTypeFlow`, `CallSiteTypeWiden`, `ConstraintSolve`, and
    /// `ConstraintSolve2` passes.
    pub constraint_solve_hm: bool,
    /// Replace `xxx_any` calls with typed variants (`_f64`, `_f32`,
    /// `_i32`, `_i64`) once operand types are known from HM inference.
    /// Runs after `constraint-solve-hm`.
    pub builtin_overload_select: bool,
    /// Inline runtime functions marked with `InlineHint::Always` at their call sites.
    /// Runs after `builtin-overload-select` and before `dead-code-elimination`.
    pub inline: bool,
    pub constant_folding: bool,
    pub cfg_simplify: bool,
    pub coroutine_lowering: bool,
    pub redundant_cast_elimination: bool,
    pub mem2reg: bool,
    pub dead_code_elimination: bool,
    /// Emit diagnostics for calls to unresolved `_any` stubs that survived
    /// all transforms (argument types could not be inferred).
    pub validate_called_stubs: bool,
    /// When enabled, the pipeline repeats all passes until none report changes.
    pub fixpoint: bool,
}

impl Default for PassConfig {
    fn default() -> Self {
        Self {
            constructor_struct_infer: true,
            constraint_solve_hm: true,
            builtin_overload_select: true,
            inline: true,
            constant_folding: true,
            cfg_simplify: true,
            coroutine_lowering: true,
            redundant_cast_elimination: true,
            mem2reg: true,
            dead_code_elimination: true,
            validate_called_stubs: true,
            fixpoint: false,
        }
    }
}

impl PassConfig {
    /// Create a config with all passes enabled except those in the skip list.
    ///
    /// Pass names correspond to `Transform::name()` values:
    /// - `"constructor-struct-infer"`
    /// - `"constraint-solve-hm"`
    /// - `"builtin-overload-select"`
    /// - `"inline"`
    /// - `"constant-folding"`
    /// - `"cfg-simplify"`
    /// - `"coroutine-lowering"`
    /// - `"redundant-cast-elimination"`
    /// - `"mem2reg"`
    /// - `"dead-code-elimination"`
    /// - `"validate-called-stubs"`
    /// - `"fixpoint"` — toggles pipeline fixpoint iteration
    ///
    /// Note: `"int-to-bool-promotion"` is an engine-specific pass injected
    /// by frontends (e.g. GameMaker) via `FrontendOutput::frontend_passes` and
    /// cannot be skipped through `PassConfig`.
    pub fn from_skip_list(skip: &[&str]) -> Self {
        let mut config = Self::default();
        for name in skip {
            config.set_skip(name);
        }
        config
    }

    /// Disable the pass identified by `name`. Returns `true` if the name was
    /// recognized, `false` otherwise (unknown names are silently ignored).
    pub fn set_skip(&mut self, name: &str) -> bool {
        match name {
            "constructor-struct-infer" => self.constructor_struct_infer = false,
            "constraint-solve-hm" => self.constraint_solve_hm = false,
            "builtin-overload-select" => self.builtin_overload_select = false,
            "inline" => self.inline = false,
            "constant-folding" => self.constant_folding = false,
            "cfg-simplify" => self.cfg_simplify = false,
            "coroutine-lowering" => self.coroutine_lowering = false,
            "redundant-cast-elimination" => self.redundant_cast_elimination = false,
            "mem2reg" => self.mem2reg = false,
            "dead-code-elimination" => self.dead_code_elimination = false,
            "validate-called-stubs" => self.validate_called_stubs = false,
            "fixpoint" => self.fixpoint = false,
            _ => return false,
        }
        true
    }
}

/// Configuration for AST lowering optimizations.
///
/// Controls which pattern-matching optimizations are applied when converting
/// structured IR to the high-level AST. Expression inlining and constant
/// propagation are always enabled — these flags control higher-level patterns.
#[derive(Debug, Clone)]
pub struct LoweringConfig {
    /// Convert single-assign if/else branches to ternary expressions.
    pub ternary: bool,
    /// Convert comparison + ternary patterns to `Math.max`/`Math.min`.
    pub minmax: bool,
    /// Convert LogicalOr/And shapes to `||`/`&&` short-circuit expressions.
    pub logical_operators: bool,
    /// Hoist loop conditions into `while (cond)` instead of
    /// `while (true) { if (!cond) break; ... }`.
    pub while_condition_hoisting: bool,
    /// System names whose `SystemCall` ops are scope-lookup ops that must be
    /// always-inline so call sites can detect and resolve them (e.g. scope
    /// lookup chains like `Field(scope_lookup, field)`).
    ///
    /// Flash sets this to `["Flash.Scope"]`; other engines leave it empty.
    pub scope_lookup_systems: Vec<String>,
    /// Wrap `ClassRef`-typed GlobalRef values with `as any` at each use site.
    ///
    /// GML OBJT class names are interchangeable with their integer object-type
    /// indices at runtime; `as any` suppresses TypeScript's `typeof ClassName`
    /// type errors when the value is used in numeric or mixed-type contexts.
    ///
    /// GML sets this to `true`; Flash and other engines leave it `false`.
    pub wrap_class_refs_as_any: bool,
    /// Rewrite `SystemCall(system, method, args)` to `MethodCall(receiver, method, args)`
    /// when `system` matches.  Used to lower engine-specific output-node calls to regular
    /// method calls before optimization passes run.
    ///
    /// Stored as `Some((system_name, receiver_var_name))`.
    /// Twine sets this to `Some(("Harlowe.H", "h"))`; all other engines leave it `None`.
    pub output_node_system: Option<(String, String)>,
    /// Rewrite `while(true) { hasNext2 ... }` loops to `for (const x of ...)`.
    ///
    /// Set to the `SystemCall` system name that provides `hasNext2`, `nextValue`,
    /// and `nextName` (e.g. `"Flash.Iterator"`).  `None` disables the rewrite.
    ///
    /// Flash sets this to `Some("Flash.Iterator")`; all other engines leave it `None`.
    pub foreach_iterator_system: Option<String>,
    /// Insert `.toString()` on `SystemCall` construct ops that produce a String
    /// result type.  In AS3, constructing `XML`/`XMLList` from a string
    /// implicitly coerces the result to a string, but the TypeScript backend
    /// rewrites `construct` to `new XML(...)` whose TS type is `XML`, not
    /// `string`.  This flag restores the correct string type at the IR-to-AST
    /// lowering boundary.
    ///
    /// Flash sets this to `true`; all other engines leave it `false`.
    pub construct_string_coerce: bool,
    /// Coerce index expressions for type-safe bracket access.
    ///
    /// When `true`, two coercions are applied in `GetIndex` emission:
    /// 1. If the collection is a `Struct` type (not Object/Class/Dictionary)
    ///    and the index is `Unknown`, the collection is wrapped with
    ///    `(collection as any)` so bracket access is allowed.
    /// 2. If the index is an XML/XMLList type, it is wrapped with
    ///    `String(index)` to coerce to a valid TS index type.
    ///
    /// Flash sets this to `true`; all other engines leave it `false`.
    pub coerce_index_types: bool,
    /// Inject `as <type>` casts on `SystemCall` results that have been narrowed
    /// by type inference.
    ///
    /// When a `SystemCall` result type has been inferred to a concrete type
    /// (e.g. `Float(64)`) but the underlying runtime function returns `unknown`
    /// in the TypeScript type signature, a cast is needed at every use site to
    /// surface the inferred type.
    ///
    /// Entries are `(system, method)` pairs whose results should be cast when
    /// the IR result type differs from `Unknown`.
    ///
    /// SugarCube sets this to `[("SugarCube.State", "get")]` so that story
    /// variables narrowed by `build_global_types` are emitted as typed values
    /// (e.g. `State.get("gold") as number`) instead of `unknown`.
    pub cast_narrowed_syscall_results_for: Vec<(String, String)>,

    /// Like `cast_narrowed_syscall_results_for`, but **only** injects casts
    /// for `Struct` and `Enum` result types.  Scalar/Array/Function types are
    /// left as-is so that existing TypeScript overloads (e.g. the named
    /// overloads on `Engine.resolve`) are not shadowed by a less-precise cast.
    ///
    /// SugarCube sets this to `[("SugarCube.Engine", "resolve")]` so that
    /// story variables inferred as structs (e.g. `$navigation`) are cast to
    /// their struct type when accessed via a bare identifier expression, while
    /// built-in global lookups like `Engine.resolve("Date")` are unaffected.
    pub cast_struct_syscall_results_for: Vec<(String, String)>,

    /// When true, any `CallIndirect` whose callee has `Type::Unknown` (i.e.
    /// would be typed `unknown` in TypeScript) is wrapped in a function-type
    /// cast before emission.  This eliminates TS2571 "Object is of type
    /// 'unknown'" errors at indirect call sites.
    ///
    /// Disabled for Flash/GML because those backends use scope-resolution
    /// rewrites (e.g. `findPropStrict` → bare name) that run *after* core
    /// emit and cannot see through a cast wrapper.  SugarCube does not have
    /// such rewrites, so it is safe to enable there.
    pub cast_unknown_indirect_callee: bool,
    /// Maps function call names to their `(system, method)` pair.
    ///
    /// When `Op::Call { func: name }` is emitted and this map contains `name`,
    /// the linear emitter lowers it to `Expr::SystemCall { system, method, args }`
    /// instead of a plain `Expr::Call`.  This allows all existing engine-specific
    /// backend rewrite passes to handle these calls unchanged.
    ///
    /// Default: empty (no intrinsic calls). Reserved for Flash/Twine frontends
    /// that emit `Op::Call` for engine syscalls.
    pub intrinsic_calls: std::collections::HashMap<String, (String, String)>,

    /// Set of FuncIds that are pure (no side effects) — used by the linear
    /// resolver to mark pure calls as deferrable.  Populated by the backend
    /// from `Module::core_builtin_fids`.
    /// Default: empty (no calls deferred).
    pub pure_fids: std::collections::HashSet<crate::ir::func::FuncId>,

    /// Map from FuncId to its canonical registry name — used by the linear
    /// emitter to look up call target names for builtin dispatch and
    /// intrinsic_calls lookups.  Populated by the backend from
    /// `runtime_registry`.  Default: empty.
    pub func_names: std::collections::HashMap<crate::ir::func::FuncId, String>,
}

impl Default for LoweringConfig {
    /// Default is the optimized preset (all optimizations enabled).
    fn default() -> Self {
        Self::optimized()
    }
}

impl LoweringConfig {
    fn literal() -> Self {
        Self {
            ternary: true,
            minmax: false,
            logical_operators: true,
            while_condition_hoisting: true,
            scope_lookup_systems: vec![],
            wrap_class_refs_as_any: false,
            output_node_system: None,
            foreach_iterator_system: None,
            construct_string_coerce: false,
            coerce_index_types: false,
            cast_narrowed_syscall_results_for: vec![],
            cast_struct_syscall_results_for: vec![],
            cast_unknown_indirect_callee: false,
            intrinsic_calls: std::collections::HashMap::new(),
            pure_fids: std::collections::HashSet::new(),
            func_names: std::collections::HashMap::new(),
        }
    }

    fn optimized() -> Self {
        Self {
            ternary: true,
            minmax: true,
            logical_operators: true,
            while_condition_hoisting: true,
            scope_lookup_systems: vec![],
            wrap_class_refs_as_any: false,
            output_node_system: None,
            foreach_iterator_system: None,
            construct_string_coerce: false,
            coerce_index_types: false,
            cast_narrowed_syscall_results_for: vec![],
            cast_struct_syscall_results_for: vec![],
            cast_unknown_indirect_callee: false,
            intrinsic_calls: std::collections::HashMap::new(),
            pure_fids: std::collections::HashSet::new(),
            func_names: std::collections::HashMap::new(),
        }
    }

    /// Populate `func_names` and `pure_fids` from a module's function table
    /// and core builtin set.  Call this before passing the config to
    /// [`crate::ir::linear::lower_function_linear`] so the linear emitter can
    /// resolve `FuncId → name` for builtin dispatch and mark builtin
    /// arithmetic calls as pure (deferrable).
    pub fn with_module(mut self, module: &crate::ir::Module) -> Self {
        self.func_names = module
            .functions
            .iter()
            .map(|(fid, func)| (fid, func.name.clone()))
            .collect();
        self.pure_fids = module.core_builtin_fids.clone();
        self
    }

    /// Populate `func_names` and `pure_fids` from a fresh core module
    /// (one that only has [`crate::ir::Module::register_core_builtins`] applied).
    ///
    /// Useful in tests that build standalone `Function` objects without a full
    /// module context but still need builtin dispatch to resolve for the linear
    /// emitter.
    pub fn with_core_module() -> Self {
        let m = crate::ir::Module::new("__core__".to_string());
        Self::default().with_module(&m)
    }
}

/// Resolve a named preset into `(PassConfig, LoweringConfig)`.
///
/// Available presets:
///
/// - **`literal`**: Faithful 1:1 translation. Skips optimization passes
///   (constant folding, DCE, cfg-simplify, redundant cast elimination) and
///   disables AST-level rewrites like Math.max/min detection. Structural
///   passes (type inference, mem2reg, coroutine lowering) still run because
///   they're needed for correct output.
///
/// - **`optimized`** (default): All transform passes and AST-level
///   optimizations enabled.
///
/// `skip_passes` are applied on top of the preset's base `PassConfig`,
/// allowing fine-grained overrides.
pub fn resolve_preset(name: &str, skip_passes: &[&str]) -> Option<(PassConfig, LoweringConfig)> {
    let (mut pass, lowering) = match name {
        "literal" => (
            PassConfig {
                // Structural passes — needed for correct output.
                constructor_struct_infer: true,
                constraint_solve_hm: true,
                builtin_overload_select: true,
                inline: true,
                coroutine_lowering: true,
                mem2reg: true,
                // Optimization passes — disabled for literal.
                constant_folding: false,
                cfg_simplify: false,
                redundant_cast_elimination: true,
                dead_code_elimination: false,
                validate_called_stubs: true,
                fixpoint: false,
            },
            LoweringConfig::literal(),
        ),
        "optimized" => (PassConfig::default(), LoweringConfig::optimized()),
        _ => return None,
    };

    // Apply --skip-pass overrides on top of the preset.
    for name in skip_passes {
        pass.set_skip(name);
    }

    Some((pass, lowering))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_enables_all() {
        let config = PassConfig::default();
        assert!(config.constraint_solve_hm);
        assert!(config.constant_folding);
        assert!(config.cfg_simplify);
        assert!(config.coroutine_lowering);
        assert!(config.dead_code_elimination);
        assert!(!config.fixpoint);
    }

    #[test]
    fn skip_list_disables_passes() {
        let config = PassConfig::from_skip_list(&["constant-folding"]);
        assert!(config.constraint_solve_hm);
        assert!(!config.constant_folding);
        assert!(config.cfg_simplify);
        assert!(config.coroutine_lowering);
        assert!(config.dead_code_elimination);
    }

    #[test]
    fn skip_list_all() {
        let config = PassConfig::from_skip_list(&[
            "constraint-solve-hm",
            "builtin-overload-select",
            "inline",
            "constant-folding",
            "cfg-simplify",
            "coroutine-lowering",
            "redundant-cast-elimination",
            "mem2reg",
            "dead-code-elimination",
            "fixpoint",
        ]);
        assert!(!config.constraint_solve_hm);
        assert!(!config.builtin_overload_select);
        assert!(!config.inline);
        assert!(!config.constant_folding);
        assert!(!config.cfg_simplify);
        assert!(!config.coroutine_lowering);
        assert!(!config.redundant_cast_elimination);
        assert!(!config.mem2reg);
        assert!(!config.dead_code_elimination);
        assert!(!config.fixpoint);
    }

    #[test]
    fn skip_list_unknown_ignored() {
        let config = PassConfig::from_skip_list(&["nonexistent"]);
        assert!(config.constraint_solve_hm);
        assert!(config.constant_folding);
        assert!(config.cfg_simplify);
    }

    #[test]
    fn preset_optimized() {
        let (pass, lowering) = resolve_preset("optimized", &[]).unwrap();
        assert!(pass.constant_folding);
        assert!(pass.cfg_simplify);
        assert!(pass.dead_code_elimination);
        assert!(pass.redundant_cast_elimination);
        assert!(lowering.minmax);
        assert!(lowering.ternary);
    }

    #[test]
    fn preset_literal() {
        let (pass, lowering) = resolve_preset("literal", &[]).unwrap();
        // Structural passes still on.
        assert!(pass.constraint_solve_hm);
        assert!(pass.mem2reg);
        assert!(pass.coroutine_lowering);
        // Optimization passes off.
        assert!(!pass.constant_folding);
        assert!(!pass.cfg_simplify);
        assert!(!pass.dead_code_elimination);
        assert!(pass.redundant_cast_elimination);
        // Lowering: faithful patterns on, rewrites off.
        assert!(lowering.ternary);
        assert!(lowering.logical_operators);
        assert!(lowering.while_condition_hoisting);
        assert!(!lowering.minmax);
    }

    #[test]
    fn preset_with_skip_overrides() {
        let (pass, _) = resolve_preset("optimized", &["mem2reg"]).unwrap();
        assert!(!pass.mem2reg);
        assert!(pass.constant_folding);
    }

    #[test]
    fn preset_unknown_returns_none() {
        assert!(resolve_preset("unknown", &[]).is_none());
    }

    fn debug_with_filter(filter: &str) -> DebugConfig {
        DebugConfig {
            dump_ir: true,
            dump_ast: false,
            dump_ir_after: None,
            function_filter: Some(filter.to_string()),
            timing: false,
        }
    }

    #[test]
    fn should_dump_no_filter() {
        let cfg = DebugConfig {
            dump_ir: true,
            dump_ast: false,
            function_filter: None,
            dump_ir_after: None,
            timing: false,
        };
        assert!(cfg.should_dump("Gun::event_step_2"));
        assert!(cfg.should_dump("anything"));
    }

    #[test]
    fn should_dump_exact_substring() {
        // Original behaviour: case-sensitive substring match.
        let cfg = debug_with_filter("event_step");
        assert!(cfg.should_dump("Gun::event_step_2"));
        assert!(!cfg.should_dump("Gun::event_draw_0"));
    }

    #[test]
    fn should_dump_case_insensitive_substring() {
        let cfg = debug_with_filter("EVENT_STEP");
        assert!(cfg.should_dump("Gun::event_step_2"));
        assert!(!cfg.should_dump("Gun::event_draw_0"));
    }

    #[test]
    fn should_dump_dot_split_parts() {
        // "Gun.step" → parts ["Gun", "step"] — both must appear in the name.
        let cfg = debug_with_filter("Gun.step");
        assert!(cfg.should_dump("Gun::event_step_2"));
        assert!(!cfg.should_dump("Bullet::event_step_2")); // "gun" not present
        assert!(!cfg.should_dump("Gun::event_draw_0")); // "step" not present
    }

    #[test]
    fn should_dump_colons_split_parts() {
        // "Gun::step" → parts ["Gun", "step"].
        let cfg = debug_with_filter("Gun::step");
        assert!(cfg.should_dump("Gun::event_step_2"));
        assert!(!cfg.should_dump("Bullet::event_step_2"));
    }

    #[test]
    fn should_dump_split_case_insensitive() {
        // Parts are compared case-insensitively.
        let cfg = debug_with_filter("GUN.STEP");
        assert!(cfg.should_dump("Gun::event_step_2"));
    }

    #[test]
    fn should_dump_free_function() {
        // Free functions have no separator.
        let cfg = debug_with_filter("scr_init");
        assert!(cfg.should_dump("scr_init"));
        assert!(!cfg.should_dump("scr_cleanup"));
    }

    #[test]
    fn should_dump_no_false_positive_on_dot_filter() {
        // "Gun.draw" must NOT match "Gun::event_step_2".
        let cfg = debug_with_filter("Gun.draw");
        assert!(!cfg.should_dump("Gun::event_step_2"));
    }
}
