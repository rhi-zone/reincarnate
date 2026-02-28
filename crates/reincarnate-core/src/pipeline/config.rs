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
            let parts: Vec<&str> = filter
                .split(['.', ':'])
                .filter(|p| !p.is_empty())
                .collect();
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
    pub type_inference: bool,
    pub call_site_flow: bool,
    pub constraint_solve: bool,
    /// Widen params narrowed by ConstraintSolve when callers pass incompatible
    /// types. Runs immediately after `constraint_solve`. Requires both
    /// `call_site_flow` and `constraint_solve` to have run for useful results,
    /// but is not gated on them (it is a no-op on already-Dynamic params).
    pub call_site_widen: bool,
    pub constant_folding: bool,
    pub cfg_simplify: bool,
    pub coroutine_lowering: bool,
    pub redundant_cast_elimination: bool,
    pub mem2reg: bool,
    pub dead_code_elimination: bool,
    /// When enabled, the pipeline repeats all passes until none report changes.
    pub fixpoint: bool,
}

impl Default for PassConfig {
    fn default() -> Self {
        Self {
            type_inference: true,
            call_site_flow: true,
            constraint_solve: true,
            call_site_widen: true,
            constant_folding: true,
            cfg_simplify: true,
            coroutine_lowering: true,
            redundant_cast_elimination: true,
            mem2reg: true,
            dead_code_elimination: true,
            fixpoint: false,
        }
    }
}

impl PassConfig {
    /// Create a config with all passes enabled except those in the skip list.
    ///
    /// Pass names correspond to `Transform::name()` values:
    /// - `"type-inference"`
    /// - `"call-site-type-flow"`
    /// - `"constraint-solve"`
    /// - `"call-site-type-widen"`
    /// - `"constant-folding"`
    /// - `"cfg-simplify"`
    /// - `"coroutine-lowering"`
    /// - `"redundant-cast-elimination"`
    /// - `"int-to-bool-promotion"`
    /// - `"mem2reg"`
    /// - `"dead-code-elimination"`
    /// - `"fixpoint"` — toggles pipeline fixpoint iteration
    pub fn from_skip_list(skip: &[&str]) -> Self {
        let mut config = Self::default();
        for name in skip {
            match *name {
                "type-inference" => config.type_inference = false,
                "call-site-type-flow" => config.call_site_flow = false,
                "constraint-solve" => config.constraint_solve = false,
                "call-site-type-widen" => config.call_site_widen = false,
                "constant-folding" => config.constant_folding = false,
                "cfg-simplify" => config.cfg_simplify = false,
                "coroutine-lowering" => config.coroutine_lowering = false,
                "redundant-cast-elimination" => config.redundant_cast_elimination = false,
                "mem2reg" => config.mem2reg = false,
                "dead-code-elimination" => config.dead_code_elimination = false,
                "fixpoint" => config.fixpoint = false,
                _ => {}
            }
        }
        config
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
        }
    }

    fn optimized() -> Self {
        Self {
            ternary: true,
            minmax: true,
            logical_operators: true,
            while_condition_hoisting: true,
        }
    }
}

/// A named preset that configures the entire pipeline: both transform passes
/// and AST lowering optimizations.
///
/// - **`literal`**: Faithful 1:1 translation. Skips optimization passes
///   (constant folding, DCE, cfg-simplify, redundant cast elimination) and
///   disables AST-level rewrites like Math.max/min detection. Structural
///   passes (type inference, mem2reg, coroutine lowering) still run because
///   they're needed for correct output.
///
/// - **`optimized`** (default): All transform passes and AST-level
///   optimizations enabled.
pub struct Preset;

impl Preset {
    /// Resolve a preset name into `(PassConfig, LoweringConfig)`.
    ///
    /// `skip_passes` are applied on top of the preset's base `PassConfig`,
    /// allowing fine-grained overrides.
    pub fn resolve(
        name: &str,
        skip_passes: &[&str],
    ) -> Option<(PassConfig, LoweringConfig)> {
        let (mut pass, lowering) = match name {
            "literal" => (
                PassConfig {
                    // Structural passes — needed for correct output.
                    type_inference: true,
                    call_site_flow: true,
                    constraint_solve: true,
                    call_site_widen: true,
                    coroutine_lowering: true,
                    mem2reg: true,
                    // Optimization passes — disabled for literal.
                    constant_folding: false,
                    cfg_simplify: false,
                    redundant_cast_elimination: true,
                    dead_code_elimination: false,
                    fixpoint: false,
                },
                LoweringConfig::literal(),
            ),
            "optimized" => (PassConfig::default(), LoweringConfig::optimized()),
            _ => return None,
        };

        // Apply --skip-pass overrides on top of the preset.
        for name in skip_passes {
            match *name {
                "type-inference" => pass.type_inference = false,
                "call-site-type-flow" => pass.call_site_flow = false,
                "constraint-solve" => pass.constraint_solve = false,
                "call-site-type-widen" => pass.call_site_widen = false,
                "constant-folding" => pass.constant_folding = false,
                "cfg-simplify" => pass.cfg_simplify = false,
                "coroutine-lowering" => pass.coroutine_lowering = false,
                "redundant-cast-elimination" => pass.redundant_cast_elimination = false,
                "mem2reg" => pass.mem2reg = false,
                "dead-code-elimination" => pass.dead_code_elimination = false,
                "fixpoint" => pass.fixpoint = false,
                _ => {}
            }
        }

        Some((pass, lowering))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_enables_all() {
        let config = PassConfig::default();
        assert!(config.type_inference);
        assert!(config.constraint_solve);
        assert!(config.constant_folding);
        assert!(config.cfg_simplify);
        assert!(config.coroutine_lowering);
        assert!(config.dead_code_elimination);
        assert!(!config.fixpoint);
    }

    #[test]
    fn skip_list_disables_passes() {
        let config = PassConfig::from_skip_list(&["constant-folding"]);
        assert!(config.type_inference);
        assert!(!config.constant_folding);
        assert!(config.cfg_simplify);
        assert!(config.coroutine_lowering);
        assert!(config.dead_code_elimination);
    }

    #[test]
    fn skip_list_all() {
        let config = PassConfig::from_skip_list(&[
            "type-inference",
            "call-site-type-flow",
            "constraint-solve",
            "call-site-type-widen",
            "constant-folding",
            "cfg-simplify",
            "coroutine-lowering",
            "redundant-cast-elimination",
            "mem2reg",
            "dead-code-elimination",
            "fixpoint",
        ]);
        assert!(!config.type_inference);
        assert!(!config.call_site_flow);
        assert!(!config.constraint_solve);
        assert!(!config.call_site_widen);
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
        assert!(config.type_inference);
        assert!(config.constant_folding);
        assert!(config.cfg_simplify);
    }

    #[test]
    fn preset_optimized() {
        let (pass, lowering) = Preset::resolve("optimized", &[]).unwrap();
        assert!(pass.constant_folding);
        assert!(pass.cfg_simplify);
        assert!(pass.dead_code_elimination);
        assert!(pass.redundant_cast_elimination);
        assert!(lowering.minmax);
        assert!(lowering.ternary);
    }

    #[test]
    fn preset_literal() {
        let (pass, lowering) = Preset::resolve("literal", &[]).unwrap();
        // Structural passes still on.
        assert!(pass.type_inference);
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
        let (pass, _) = Preset::resolve("optimized", &["mem2reg"]).unwrap();
        assert!(!pass.mem2reg);
        assert!(pass.constant_folding);
    }

    #[test]
    fn preset_unknown_returns_none() {
        assert!(Preset::resolve("unknown", &[]).is_none());
    }

    fn debug_with_filter(filter: &str) -> DebugConfig {
        DebugConfig {
            dump_ir: true,
            dump_ast: false,
            dump_ir_after: None,
            function_filter: Some(filter.to_string()),
        }
    }

    #[test]
    fn should_dump_no_filter() {
        let cfg = DebugConfig { dump_ir: true, dump_ast: false, function_filter: None, dump_ir_after: None };
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
        assert!(!cfg.should_dump("Gun::event_draw_0"));    // "step" not present
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
