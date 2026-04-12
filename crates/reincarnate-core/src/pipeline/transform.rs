use std::collections::HashSet;

use super::config::DebugConfig;
use crate::error::CoreError;
use crate::ir::func::FuncId;
use crate::ir::Module;

/// Result of applying a transform pass.
pub struct TransformResult {
    pub module: Module,
    /// Whether the pass modified the module.
    pub changed: bool,
    /// Which functions were modified in this pass. Empty means none changed.
    /// Used by fixpoint mode to limit re-runs to dirty functions only.
    pub changed_funcs: HashSet<FuncId>,
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
    ///
    /// `dirty` is an optional set of function IDs that were modified by a
    /// previous pass. `None` means "process all functions" (first iteration or
    /// pass doesn't support incremental). `Some(set)` means "only process
    /// functions in this set".
    fn apply(
        &self,
        module: Module,
        dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError>;

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

    fn apply(
        &self,
        module: Module,
        dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        (**self).apply(module, dirty)
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

/// Descriptor for a single transform pass in the standard pipeline.
///
/// Used by [`TransformPipeline::from_descriptors`] to build a pipeline
/// with dependency-driven ordering and invalidation expansion.
pub struct PassDescriptor {
    /// Kebab-case name matching [`Transform::name()`].
    pub name: &'static str,
    /// Create a fresh instance of this pass.
    pub factory: fn() -> Box<dyn Transform>,
    /// Whether this pass is enabled by the given config.
    pub config_enabled: fn(&super::config::PassConfig) -> bool,
}

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

    /// Build a pipeline by topo-sorting enabled passes and expanding invalidations.
    ///
    /// Enabled passes (those where `descriptor.config_enabled(config)` is true)
    /// are sorted in topological order according to their `requires()` declarations,
    /// with ties broken by registration order in `descriptors`. Invalidation
    /// expansion then inserts re-runs of invalidated passes where needed so that
    /// every pass that `requires` an invalidated pass gets a fresh run.
    pub fn from_descriptors(
        descriptors: &[PassDescriptor],
        config: &super::config::PassConfig,
    ) -> Self {
        let enabled: Vec<Box<dyn Transform>> = descriptors
            .iter()
            .filter(|d| (d.config_enabled)(config))
            .map(|d| (d.factory)())
            .collect();

        let ordered = topo_sort(enabled, descriptors);
        let expanded = expand_invalidations(ordered, descriptors);

        Self {
            transforms: expanded,
            fixpoint: config.fixpoint,
        }
    }

    /// Returns the name of each pass in execution order.
    ///
    /// Used by the CLI to validate `--dump-ir-after` arguments.
    pub fn pass_names(&self) -> Vec<&str> {
        self.transforms.iter().map(|t| t.name()).collect()
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

        let run_once_names: std::collections::HashSet<&str> = self
            .transforms
            .iter()
            .filter(|t| t.run_once())
            .map(|t| t.name())
            .collect();

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
                assert!(
                    !run_once_names.contains(inv),
                    "pass {:?} invalidates {:?} but {:?} is run_once — structural passes must not be invalidated",
                    t.name(),
                    inv,
                    inv,
                );
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
            let mut dirty: Option<HashSet<FuncId>> = None; // None = all dirty on first iteration

            let mut first_iteration = true;
            let mut iterations_remaining = MAX_FIXPOINT_ITERATIONS;
            loop {
                assert!(
                    iterations_remaining > 0,
                    "fixpoint did not converge after {MAX_FIXPOINT_ITERATIONS} iterations"
                );
                iterations_remaining -= 1;
                let mut any_changed = false;
                let mut next_dirty: HashSet<FuncId> = HashSet::new();
                // Whether any module-level (non-per-function-tracked) change occurred.
                // When true, we pass None to all passes in the next iteration so
                // they run without filtering, even if next_dirty is empty.
                let mut module_level_changed = false;

                for transform in &self.transforms {
                    if !first_iteration && transform.run_once() {
                        continue;
                    }
                    // On first iteration pass None (all functions). After that pass dirty set,
                    // but only if no module-level change occurred (which would invalidate
                    // the per-function dirty tracking).
                    let pass_dirty = if first_iteration || module_level_changed {
                        None
                    } else {
                        dirty.as_ref().map(|d| d as &HashSet<FuncId>)
                    };
                    // Skip pass entirely if dirty set is empty and no module-level changes
                    // (nothing for it to do).
                    if pass_dirty.is_some_and(|d| d.is_empty()) {
                        continue;
                    }
                    let t0 = if debug.timing {
                        Some(std::time::Instant::now())
                    } else {
                        None
                    };
                    let result = transform.apply(module, pass_dirty)?;
                    if let Some(t0) = t0 {
                        let iter = MAX_FIXPOINT_ITERATIONS - iterations_remaining;
                        eprintln!(
                            "[timing] iter {}: {}: {}ms",
                            iter,
                            transform.name(),
                            t0.elapsed().as_millis()
                        );
                    }
                    if result.changed {
                        if result.changed_funcs.is_empty() {
                            // Pass changed something but didn't track which functions.
                            // Mark module-level change so next iteration doesn't filter.
                            module_level_changed = true;
                        } else {
                            next_dirty.extend(result.changed_funcs.iter().copied());
                        }
                    }
                    any_changed |= result.changed;
                    module = result.module;
                }
                first_iteration = false;
                if !any_changed {
                    break;
                }
                // If a module-level change occurred, reset dirty to None so the
                // next iteration runs all passes without filtering.
                if module_level_changed {
                    dirty = None;
                } else {
                    dirty = Some(next_dirty);
                }
            }
        } else {
            for transform in &self.transforms {
                let t0 = if debug.timing {
                    Some(std::time::Instant::now())
                } else {
                    None
                };
                module = transform.apply(module, None)?.module;
                if let Some(t0) = t0 {
                    eprintln!(
                        "[timing] {}: {}ms",
                        transform.name(),
                        t0.elapsed().as_millis()
                    );
                }
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

/// Sort enabled passes into topological order using Kahn's algorithm.
///
/// Edges are derived from `requires()` declarations: if `B.requires()` contains
/// `A.name()` and A is enabled, then A must appear before B.
///
/// Ties in in-degree-0 candidates are broken by registration order in
/// `descriptors`, so the output is deterministic and matches the natural
/// pipeline order when no dependencies force a different arrangement.
///
/// Passes whose `requires()` entries are all absent from the enabled set
/// are treated as having no unsatisfied dependencies (the dependency is
/// simply not in the pipeline and is ignored). This allows passing a subset
/// of the full pipeline without errors.
///
/// Panics if a dependency cycle is detected among the enabled passes.
fn topo_sort(
    passes: Vec<Box<dyn Transform>>,
    descriptors: &[PassDescriptor],
) -> Vec<Box<dyn Transform>> {
    let desc_idx: std::collections::HashMap<&str, usize> = descriptors
        .iter()
        .enumerate()
        .map(|(i, d)| (d.name, i))
        .collect();

    let enabled_names: std::collections::HashSet<String> =
        passes.iter().map(|p| p.name().to_string()).collect();

    let n = passes.len();
    let mut result = Vec::with_capacity(n);
    let mut remaining: Vec<Option<Box<dyn Transform>>> = passes.into_iter().map(Some).collect();

    let mut added_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        // Find all passes whose `requires()` entries that are enabled are
        // all already in `added_names`.
        let mut candidates: Vec<usize> = (0..remaining.len())
            .filter(|&i| {
                if let Some(p) = &remaining[i] {
                    p.requires()
                        .iter()
                        .filter(|r| enabled_names.contains(**r))
                        .all(|r| added_names.contains(*r))
                } else {
                    false
                }
            })
            .collect();

        if candidates.is_empty() {
            break;
        }

        // Break ties by descriptor registration order.
        candidates.sort_by_key(|&i| {
            desc_idx
                .get(remaining[i].as_ref().unwrap().name())
                .copied()
                .unwrap_or(usize::MAX)
        });

        for i in candidates {
            if let Some(p) = remaining[i].take() {
                added_names.insert(p.name().to_string());
                result.push(p);
            }
        }
    }

    // Check for cycles: any remaining Some entries have unresolvable deps.
    let leftover: Vec<String> = remaining
        .iter()
        .filter_map(|opt| opt.as_ref().map(|p| p.name().to_string()))
        .collect();
    if !leftover.is_empty() {
        panic!(
            "dependency cycle detected among passes: {}",
            leftover.join(", ")
        );
    }

    result
}

/// Expand invalidations: for each pass P that declares `invalidates(Q)`,
/// if any later pass R requires Q but Q does not appear between P and R,
/// insert a fresh Q pass before R.
///
/// Repeats until no more insertions are needed.
fn expand_invalidations(
    mut passes: Vec<Box<dyn Transform>>,
    descriptors: &[PassDescriptor],
) -> Vec<Box<dyn Transform>> {
    let mut changed = true;
    while changed {
        changed = false;
        'outer: for i in 0..passes.len() {
            for inv_name in passes[i].invalidates() {
                // Find the first position j > i where passes[j].requires()
                // contains inv_name.
                let first_needer = passes[i + 1..]
                    .iter()
                    .position(|p| p.requires().contains(inv_name));

                let Some(rel_pos) = first_needer else {
                    continue;
                };
                let abs_first_needer = i + 1 + rel_pos;

                // If inv_name already appears between i and abs_first_needer,
                // the requirement is already satisfied.
                let already_present = passes[i + 1..abs_first_needer]
                    .iter()
                    .any(|p| p.name() == *inv_name);

                if already_present {
                    continue;
                }

                // Look up the factory for inv_name.
                let Some(factory) = descriptors
                    .iter()
                    .find(|d| d.name == *inv_name)
                    .map(|d| d.factory)
                else {
                    // inv_name is not in the registry (disabled or unknown) — skip.
                    continue;
                };

                let fresh = factory();
                assert!(
                    !fresh.run_once(),
                    "pass {:?} invalidates {:?} but {:?} is run_once — it cannot be re-inserted",
                    passes[i].name(),
                    inv_name,
                    inv_name,
                );
                let fresh_requires: Vec<&str> = fresh.requires().to_vec();

                // Insert after the last dep of fresh in (i, abs_first_needer),
                // so the inserted pass's own requires() are already satisfied.
                let insert_at = {
                    let mut best = i + 1;
                    for dep_name in &fresh_requires {
                        for j in (i + 1..abs_first_needer).rev() {
                            if passes[j].name() == *dep_name {
                                best = best.max(j + 1);
                                break;
                            }
                        }
                    }
                    best
                };

                passes.insert(insert_at, fresh);
                changed = true;
                break 'outer;
            }
        }
    }
    passes
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

        fn apply(
            &self,
            module: Module,
            _dirty: Option<&HashSet<FuncId>>,
        ) -> Result<TransformResult, CoreError> {
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
                changed_funcs: HashSet::new(),
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

        fn apply(
            &self,
            module: Module,
            _dirty: Option<&HashSet<FuncId>>,
        ) -> Result<TransformResult, CoreError> {
            Ok(TransformResult {
                module,
                changed: false,
                changed_funcs: HashSet::new(),
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
