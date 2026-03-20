use super::config::DebugConfig;
use crate::error::CoreError;
use crate::ir::Module;

/// Result of applying a transform pass.
pub struct TransformResult {
    pub module: Module,
    /// Whether the pass modified the module.
    pub changed: bool,
}

/// Output of the transform pipeline.
pub struct PipelineOutput {
    pub module: Module,
    /// `true` when the pipeline was stopped early by `--dump-ir-after`.
    /// The caller should skip the backend (structurize/emit) step.
    pub stopped_early: bool,
}

/// Transform trait — a pass that transforms IR modules.
///
/// Examples: type inference, coroutine lowering, dead code elimination,
/// constant folding, inlining.
pub trait Transform {
    /// Name of this transform pass.
    fn name(&self) -> &str;

    /// Apply this transform to a module, returning the transformed module
    /// and whether any changes were made.
    fn apply(&self, module: Module) -> Result<TransformResult, CoreError>;

    /// If true, the pipeline skips this pass on fixpoint iterations after the
    /// first. Use for interprocedural passes whose evidence becomes circular
    /// when repeated with bidirectional constraint solving.
    fn run_once(&self) -> bool {
        false
    }

    /// Pass names that must appear before this pass in the pipeline.
    /// The pipeline validates this at startup and panics if violated.
    fn requires(&self) -> &[&str] {
        &[]
    }

    /// Pass names whose results this pass invalidates (i.e. that may need
    /// to re-run after this pass). Used for documentation and validation —
    /// the pipeline checks that no invalidated pass appears *only* before
    /// this pass without a later re-run.
    fn invalidates(&self) -> &[&str] {
        &[]
    }
}

/// Marker trait for transforms that are provably IR-only: stateless, no external I/O,
/// IR in → IR out. Required for passes injected by frontends via
/// `FrontendOutput::frontend_passes` to enforce Law 1 (Pipeline Stage Isolation) at the
/// type level.
///
/// Implementing this trait is a contract: the pass may only read and write IR data
/// structures. It must not access filesystem, network, global mutable state, or any
/// non-IR channel.
pub trait PureIrPass: Transform {}

/// Allow `Box<dyn PureIrPass>` to be used wherever `Box<dyn Transform>` is expected
/// by forwarding all `Transform` method calls through the inner trait object.
impl Transform for Box<dyn PureIrPass> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn apply(&self, module: Module) -> Result<TransformResult, CoreError> {
        (**self).apply(module)
    }

    fn run_once(&self) -> bool {
        (**self).run_once()
    }

    fn requires(&self) -> &[&str] {
        (**self).requires()
    }

    fn invalidates(&self) -> &[&str] {
        (**self).invalidates()
    }
}

/// Maximum number of fixpoint iterations before giving up.
const MAX_FIXPOINT_ITERATIONS: usize = 100;

/// Valid pass names for `--dump-ir-after`, in pipeline order.
///
/// Order must match `default_pipeline` in `transforms/mod.rs` exactly —
/// the pipeline stops after the named pass, so a wrong order causes
/// `--dump-ir-after <pass>` to stop at the wrong point.
pub const VALID_PASS_NAMES: &[&str] = &[
    "frontend",
    "constructor-struct-infer",
    "type-inference",
    "call-site-type-flow",
    "constraint-solve",
    "call-site-type-widen",
    "call-site-arity-widen",
    "constant-folding",
    "cfg-simplify",
    "coroutine-lowering",
    "mem2reg",
    "redundant-cast-elimination",
    "dead-code-elimination",
];

/// An ordered sequence of transforms to apply.
pub struct TransformPipeline {
    transforms: Vec<Box<dyn Transform>>,
    fixpoint: bool,
}

impl TransformPipeline {
    pub fn new() -> Self {
        Self {
            transforms: Vec::new(),
            fixpoint: false,
        }
    }

    pub fn add(&mut self, transform: Box<dyn Transform>) {
        self.transforms.push(transform);
    }

    /// Add a [`PureIrPass`] to the pipeline.
    ///
    /// `Box<dyn PureIrPass>` implements `Transform` via the blanket impl above,
    /// so we wrap it in a second box to satisfy `Box<dyn Transform>`.
    pub fn add_pure(&mut self, pass: Box<dyn PureIrPass>) {
        self.transforms.push(Box::new(pass));
    }

    /// Enable fixpoint iteration: re-run the entire pipeline until no pass
    /// reports changes, or until the iteration cap is reached.
    pub fn set_fixpoint(&mut self, enabled: bool) {
        self.fixpoint = enabled;
    }

    /// Validate that all `requires()` and `invalidates()` declarations are
    /// satisfied by the current pipeline ordering.
    ///
    /// Panics if:
    /// - A pass declares a `requires` dependency on a pass that does not
    ///   appear earlier in the pipeline.
    /// - A pass declares that it `invalidates` another pass, but that pass
    ///   appears only before the invalidating pass with no later re-run.
    ///
    /// This is a developer-time check — violations are configuration errors
    /// in pass declarations, not runtime conditions.
    pub fn validate_ordering(&self) {
        // Build name → list of positions (a pass can appear multiple times,
        // e.g. ConstantFolding runs twice).
        let mut name_positions: std::collections::HashMap<&str, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, t) in self.transforms.iter().enumerate() {
            name_positions.entry(t.name()).or_default().push(i);
        }

        for (i, t) in self.transforms.iter().enumerate() {
            // Check requires: every required pass must have at least one
            // occurrence before position i.
            for &req in t.requires() {
                let satisfied = name_positions
                    .get(req)
                    .is_some_and(|positions| positions.iter().any(|&p| p < i));
                assert!(
                    satisfied,
                    "pass {:?} requires {:?} but it does not appear earlier in the pipeline",
                    t.name(),
                    req,
                );
            }

            // Check invalidates: if this pass invalidates another pass, that
            // pass must either (a) not be in the pipeline at all, or (b) have
            // at least one occurrence after position i.
            for &inv in t.invalidates() {
                if let Some(positions) = name_positions.get(inv) {
                    // The invalidated pass is in the pipeline. It must have a
                    // re-run after this pass.
                    let has_later = positions.iter().any(|&p| p > i);
                    assert!(
                        has_later,
                        "pass {:?} invalidates {:?} but {:?} has no later re-run in the pipeline",
                        t.name(),
                        inv,
                        inv,
                    );
                }
            }
        }
    }

    /// Run all transforms in order on the given module.
    ///
    /// When fixpoint mode is enabled, the pipeline repeats until a full
    /// pass over all transforms produces no changes.
    pub fn run(&self, module: Module) -> Result<Module, CoreError> {
        Ok(self.run_with_debug(module, &DebugConfig::default())?.module)
    }

    /// Run the pipeline, honouring debug configuration.
    ///
    /// When `debug.dump_ir_after` is `Some(pass_name)`:
    /// - The special value `"frontend"` dumps the module before any transforms
    ///   and returns immediately.
    /// - Otherwise, the pipeline runs transforms one-by-one and stops after the
    ///   named pass, dumping IR (filtered by `debug.function_filter`) and
    ///   returning with `stopped_early = true`.
    /// - If the named pass is not in the pipeline (e.g. it was disabled via
    ///   `--skip-pass`), the pipeline runs to completion and returns
    ///   `stopped_early = false` — the caller can emit a warning.
    pub fn run_with_debug(
        &self,
        mut module: Module,
        debug: &DebugConfig,
    ) -> Result<PipelineOutput, CoreError> {
        self.validate_ordering();

        // Special case: dump raw IR before any transforms.
        if debug.dump_ir_after.as_deref() == Some("frontend") {
            dump_ir_functions(&module, debug);
            return Ok(PipelineOutput {
                module,
                stopped_early: true,
            });
        }

        let stop_after = debug.dump_ir_after.as_deref();

        if self.fixpoint {
            // In fixpoint mode we can't meaningfully stop mid-iteration, so we
            // run to completion and ignore `dump_ir_after`.  The non-fixpoint
            // single-pass path below handles the interactive debug workflow.
            for iteration in 0..MAX_FIXPOINT_ITERATIONS {
                let mut any_changed = false;
                for transform in &self.transforms {
                    if iteration > 0 && transform.run_once() {
                        continue;
                    }
                    let result = transform.apply(module)?;
                    any_changed |= result.changed;
                    module = result.module;
                }
                if !any_changed {
                    break;
                }
            }
        } else {
            for transform in &self.transforms {
                module = transform.apply(module)?.module;
                if stop_after == Some(transform.name()) {
                    dump_ir_functions(&module, debug);
                    return Ok(PipelineOutput {
                        module,
                        stopped_early: true,
                    });
                }
            }
        }

        // Compact instruction arenas: remove dead instructions left behind by
        // transforms (Mem2Reg, DCE, etc.) so downstream consumers can safely
        // iterate the arena without encountering orphaned entries.
        for func in module.functions.values_mut() {
            func.compact_insts();
        }

        Ok(PipelineOutput {
            module,
            stopped_early: false,
        })
    }
}

/// Dump IR for all functions in `module` that pass the debug filter.
fn dump_ir_functions(module: &Module, debug: &DebugConfig) {
    for (id, func) in module.functions.iter() {
        let name = module.func_name(id);
        if debug.should_dump(name) {
            eprintln!("=== IR: {name} ===");
            // Use write_function_with_name for proper named output.
            struct NamedFunc<'a>(&'a super::super::ir::Function, &'a str);
            impl std::fmt::Display for NamedFunc<'_> {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    super::super::ir::printer::write_function_with_name(f, self.0, self.1)
                }
            }
            eprintln!("{}", NamedFunc(func, name));
            eprintln!("=== end IR ===\n");
        }
    }
}

impl Default for TransformPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A mock transform that reports `changed` for its first N calls, then stops.
    struct MockTransform {
        name: &'static str,
        changes_left: AtomicUsize,
    }

    impl MockTransform {
        fn new(name: &'static str, num_changes: usize) -> Self {
            Self {
                name,
                changes_left: AtomicUsize::new(num_changes),
            }
        }
    }

    impl Transform for MockTransform {
        fn name(&self) -> &str {
            self.name
        }

        fn apply(&self, module: Module) -> Result<TransformResult, CoreError> {
            let prev = self
                .changes_left
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                    if n > 0 {
                        Some(n - 1)
                    } else {
                        None
                    }
                });
            Ok(TransformResult {
                module,
                changed: prev.is_ok(),
            })
        }
    }

    #[test]
    fn single_pass_no_fixpoint() {
        let module = Module::new("test".into());
        let mut pipeline = TransformPipeline::new();
        pipeline.add(Box::new(MockTransform::new("a", 5)));
        // Without fixpoint, the transform runs exactly once.
        let _result = pipeline.run(module).unwrap();
        let mock = pipeline.transforms[0].as_ref() as *const dyn Transform as *const MockTransform;
        // Safety: we know the concrete type.
        let remaining = unsafe { (*mock).changes_left.load(Ordering::SeqCst) };
        assert_eq!(remaining, 4); // ran once, decremented from 5 to 4
    }

    #[test]
    fn fixpoint_runs_until_stable() {
        let module = Module::new("test".into());
        let mut pipeline = TransformPipeline::new();
        pipeline.add(Box::new(MockTransform::new("a", 3)));
        pipeline.set_fixpoint(true);
        let _result = pipeline.run(module).unwrap();
        let mock = pipeline.transforms[0].as_ref() as *const dyn Transform as *const MockTransform;
        // After 3 changes + 1 stable iteration = 4 calls total. changes_left = 0.
        let remaining = unsafe { (*mock).changes_left.load(Ordering::SeqCst) };
        assert_eq!(remaining, 0);
    }

    #[test]
    fn fixpoint_with_multiple_passes() {
        let module = Module::new("test".into());
        let mut pipeline = TransformPipeline::new();
        // Pass A changes twice, pass B changes once.
        // Iteration 1: A changes (2→1), B changes (1→0) → any_changed=true
        // Iteration 2: A changes (1→0), B stable → any_changed=true
        // Iteration 3: A stable, B stable → done
        pipeline.add(Box::new(MockTransform::new("a", 2)));
        pipeline.add(Box::new(MockTransform::new("b", 1)));
        pipeline.set_fixpoint(true);
        let _result = pipeline.run(module).unwrap();

        let mock_a =
            pipeline.transforms[0].as_ref() as *const dyn Transform as *const MockTransform;
        let mock_b =
            pipeline.transforms[1].as_ref() as *const dyn Transform as *const MockTransform;
        let remaining_a = unsafe { (*mock_a).changes_left.load(Ordering::SeqCst) };
        let remaining_b = unsafe { (*mock_b).changes_left.load(Ordering::SeqCst) };
        assert_eq!(remaining_a, 0);
        assert_eq!(remaining_b, 0);
    }

    /// A mock transform with configurable `requires` and `invalidates`.
    struct DeclMock {
        name: &'static str,
        requires: &'static [&'static str],
        invalidates: &'static [&'static str],
    }

    impl Transform for DeclMock {
        fn name(&self) -> &str {
            self.name
        }

        fn apply(&self, module: Module) -> Result<TransformResult, CoreError> {
            Ok(TransformResult {
                module,
                changed: false,
            })
        }

        fn requires(&self) -> &[&str] {
            self.requires
        }

        fn invalidates(&self) -> &[&str] {
            self.invalidates
        }
    }

    #[test]
    fn validate_ordering_satisfied() {
        let mut pipeline = TransformPipeline::new();
        pipeline.add(Box::new(DeclMock {
            name: "a",
            requires: &[],
            invalidates: &[],
        }));
        pipeline.add(Box::new(DeclMock {
            name: "b",
            requires: &["a"],
            invalidates: &[],
        }));
        pipeline.validate_ordering(); // should not panic
    }

    #[test]
    #[should_panic(expected = "requires \"a\" but it does not appear earlier")]
    fn validate_ordering_missing_requires() {
        let mut pipeline = TransformPipeline::new();
        pipeline.add(Box::new(DeclMock {
            name: "b",
            requires: &["a"],
            invalidates: &[],
        }));
        pipeline.validate_ordering();
    }

    #[test]
    fn validate_ordering_invalidates_with_rerun() {
        let mut pipeline = TransformPipeline::new();
        pipeline.add(Box::new(DeclMock {
            name: "fold",
            requires: &[],
            invalidates: &[],
        }));
        pipeline.add(Box::new(DeclMock {
            name: "mem2reg",
            requires: &[],
            invalidates: &["fold"],
        }));
        pipeline.add(Box::new(DeclMock {
            name: "fold",
            requires: &[],
            invalidates: &[],
        }));
        pipeline.validate_ordering(); // should not panic — fold re-runs after mem2reg
    }

    #[test]
    #[should_panic(expected = "invalidates \"fold\" but \"fold\" has no later re-run")]
    fn validate_ordering_invalidates_without_rerun() {
        let mut pipeline = TransformPipeline::new();
        pipeline.add(Box::new(DeclMock {
            name: "fold",
            requires: &[],
            invalidates: &[],
        }));
        pipeline.add(Box::new(DeclMock {
            name: "mem2reg",
            requires: &[],
            invalidates: &["fold"],
        }));
        pipeline.validate_ordering();
    }

    #[test]
    fn validate_ordering_invalidates_absent_pass() {
        // If the invalidated pass is not in the pipeline at all, that's fine.
        let mut pipeline = TransformPipeline::new();
        pipeline.add(Box::new(DeclMock {
            name: "mem2reg",
            requires: &[],
            invalidates: &["fold"],
        }));
        pipeline.validate_ordering(); // should not panic
    }
}
