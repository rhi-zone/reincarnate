pub mod builtin_overload_select;
pub mod cfg_simplify;
pub mod const_fold;
pub mod constraint_collect;
pub mod constraint_solve_hm;
pub mod constructor_struct_infer;
pub mod coroutine_lower;
pub mod dce;
pub mod inline;
pub mod int_to_bool;
pub mod mem2reg;
pub mod red_cast_elim;
pub mod redundant_inherited_field;
pub mod util;
pub mod validate_called_stubs;
pub mod validate_no_escaped_type_vars;

pub use builtin_overload_select::BuiltinOverloadSelect;
pub use cfg_simplify::CfgSimplify;
pub use const_fold::ConstantFolding;
pub use constraint_solve_hm::ConstraintSolveHM;
pub use constructor_struct_infer::ConstructorStructInfer;
pub use coroutine_lower::CoroutineLowering;
pub use dce::DeadCodeElimination;
pub use inline::Inline;
pub use int_to_bool::IntToBoolPromotion;
pub use mem2reg::Mem2Reg;
pub use red_cast_elim::RedundantCastElimination;
pub use redundant_inherited_field::RedundantInheritedFieldPrune;
pub use validate_called_stubs::ValidateCalledStubs;
pub use validate_no_escaped_type_vars::ValidateNoEscapedTypeVars;

use crate::pipeline::{PassConfig, PassDescriptor, TransformPipeline};

/// All known transform passes in the standard pipeline, in preferred
/// registration order.
///
/// The actual execution order is determined by topo-sort on `requires()`
/// declarations plus invalidation expansion. Registration order is used only
/// as a tie-breaker when multiple passes have in-degree zero simultaneously.
///
/// Note: `IntToBoolPromotion` and `RedundantInheritedFieldPrune` are
/// frontend-injected via `FrontendOutput::frontend_passes` and are NOT
/// included here.
pub fn all_passes() -> Vec<PassDescriptor> {
    vec![
        PassDescriptor {
            name: "constructor-struct-infer",
            factory: || Box::new(ConstructorStructInfer),
            config_enabled: |c| c.constructor_struct_infer,
        },
        PassDescriptor {
            name: "constraint-solve-hm",
            factory: || Box::new(ConstraintSolveHM),
            config_enabled: |c| c.constraint_solve_hm,
        },
        PassDescriptor {
            name: "builtin-overload-select",
            factory: || Box::new(BuiltinOverloadSelect),
            config_enabled: |c| c.builtin_overload_select,
        },
        PassDescriptor {
            name: "inline",
            factory: || Box::new(Inline),
            config_enabled: |c| c.inline,
        },
        PassDescriptor {
            name: "constant-folding",
            factory: || Box::new(ConstantFolding),
            config_enabled: |c| c.constant_folding,
        },
        PassDescriptor {
            name: "cfg-simplify",
            factory: || Box::new(CfgSimplify),
            config_enabled: |c| c.cfg_simplify,
        },
        PassDescriptor {
            name: "coroutine-lowering",
            factory: || Box::new(CoroutineLowering),
            config_enabled: |c| c.coroutine_lowering,
        },
        PassDescriptor {
            name: "mem2reg",
            factory: || Box::new(Mem2Reg),
            config_enabled: |c| c.mem2reg,
        },
        PassDescriptor {
            name: "redundant-cast-elimination",
            factory: || Box::new(RedundantCastElimination),
            config_enabled: |c| c.redundant_cast_elimination,
        },
        PassDescriptor {
            name: "dead-code-elimination",
            factory: || Box::new(DeadCodeElimination),
            config_enabled: |c| c.dead_code_elimination,
        },
        PassDescriptor {
            name: "validate-called-stubs",
            factory: || Box::new(ValidateCalledStubs),
            config_enabled: |c| c.validate_called_stubs,
        },
        PassDescriptor {
            name: "validate-no-escaped-type-vars",
            factory: || Box::new(ValidateNoEscapedTypeVars),
            config_enabled: |c| c.validate_no_escaped_type_vars,
        },
    ]
}

/// Build a transform pipeline based on the given pass configuration.
///
/// Enabled passes are topo-sorted by their `requires()` declarations and
/// expanded by their `invalidates()` declarations, so re-runs (e.g. the
/// second ConstraintSolveHM and ConstantFolding after Mem2Reg) are inserted
/// automatically without any hardcoded ordering logic.
pub fn build_pipeline(config: &PassConfig) -> TransformPipeline {
    let descriptors = all_passes();
    TransformPipeline::from_descriptors(&descriptors, config)
}

#[cfg(test)]
mod interaction_tests;
#[cfg(test)]
mod stress_tests;

#[cfg(test)]
mod pipeline_order_tests {
    use super::*;

    /// Verify that the default pipeline produces the expected pass order,
    /// including automatic re-runs of ConstraintSolveHM and ConstantFolding
    /// (after Mem2Reg) via invalidation expansion.
    #[test]
    fn default_pipeline_pass_order() {
        let config = PassConfig::default();
        let pipeline = build_pipeline(&config);
        let names = pipeline.pass_names();

        // Find the positions of key passes.
        let pos_mem2reg = names
            .iter()
            .position(|&n| n == "mem2reg")
            .expect("mem2reg missing");
        let pos_hm: Vec<usize> = names
            .iter()
            .enumerate()
            .filter(|(_, &n)| n == "constraint-solve-hm")
            .map(|(i, _)| i)
            .collect();
        let pos_cf: Vec<usize> = names
            .iter()
            .enumerate()
            .filter(|(_, &n)| n == "constant-folding")
            .map(|(i, _)| i)
            .collect();

        // ConstraintSolveHM must appear twice: once before Mem2Reg, once after.
        assert_eq!(
            pos_hm.len(),
            2,
            "expected 2 constraint-solve-hm runs, got {:?}",
            pos_hm
        );
        assert!(
            pos_hm[0] < pos_mem2reg,
            "first constraint-solve-hm must be before mem2reg"
        );
        assert!(
            pos_hm[1] > pos_mem2reg,
            "second constraint-solve-hm must be after mem2reg"
        );

        // ConstantFolding must appear twice: once before Mem2Reg, once after.
        assert_eq!(
            pos_cf.len(),
            2,
            "expected 2 constant-folding runs, got {:?}",
            pos_cf
        );
        assert!(
            pos_cf[0] < pos_mem2reg,
            "first constant-folding must be before mem2reg"
        );
        assert!(
            pos_cf[1] > pos_mem2reg,
            "second constant-folding must be after mem2reg"
        );

        // BuiltinOverloadSelect must appear after the first ConstraintSolveHM
        // and before Mem2Reg (run_once, so appears exactly once).
        let pos_bos = names
            .iter()
            .position(|&n| n == "builtin-overload-select")
            .expect("builtin-overload-select missing");
        assert!(
            pos_bos > pos_hm[0],
            "builtin-overload-select must be after first constraint-solve-hm"
        );
        assert!(
            pos_bos < pos_mem2reg,
            "builtin-overload-select must be before mem2reg"
        );

        // RedundantCastElimination must appear after the last ConstraintSolveHM
        // and last ConstantFolding.
        let pos_rce = names
            .iter()
            .position(|&n| n == "redundant-cast-elimination")
            .expect("redundant-cast-elimination missing");
        assert!(
            pos_rce > pos_hm[1],
            "rce must be after last constraint-solve-hm"
        );
        assert!(
            pos_rce > pos_cf[1],
            "rce must be after last constant-folding"
        );

        // DeadCodeElimination must appear before both validation passes.
        let pos_dce = names
            .iter()
            .position(|&n| n == "dead-code-elimination")
            .expect("dead-code-elimination missing");

        // ValidateCalledStubs must appear after DCE.
        let pos_vcs = names
            .iter()
            .position(|&n| n == "validate-called-stubs")
            .expect("validate-called-stubs missing");
        assert!(
            pos_vcs > pos_dce,
            "validate-called-stubs must be after dead-code-elimination"
        );

        // ValidateNoEscapedTypeVars must appear after DCE.
        let pos_vnetv = names
            .iter()
            .position(|&n| n == "validate-no-escaped-type-vars")
            .expect("validate-no-escaped-type-vars missing");
        assert!(
            pos_vnetv > pos_dce,
            "validate-no-escaped-type-vars must be after dead-code-elimination"
        );

        // Both validation passes must appear at the end (after the last HM run).
        assert!(
            pos_vcs > pos_hm[1],
            "validate-called-stubs must be after last constraint-solve-hm"
        );
        assert!(
            pos_vnetv > pos_hm[1],
            "validate-no-escaped-type-vars must be after last constraint-solve-hm"
        );

        // Inline must appear after builtin-overload-select and before dce (run_once).
        let pos_inline = names
            .iter()
            .position(|&n| n == "inline")
            .expect("inline missing");
        assert!(
            pos_inline > pos_bos,
            "inline must be after builtin-overload-select"
        );
        assert!(
            pos_inline < pos_dce,
            "inline must be before dead-code-elimination"
        );

        // validate_ordering must not panic.
        pipeline.validate_ordering();
    }
}
