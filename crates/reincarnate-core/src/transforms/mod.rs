pub mod call_site_flow;
pub mod call_site_widen;
pub mod cfg_simplify;
pub mod const_fold;
pub mod constraint_collect;
pub mod constraint_solve;
pub mod constraint_solve2;
pub mod constructor_struct_infer;
pub mod coroutine_lower;
pub mod dce;
pub mod int_to_bool;
pub mod mem2reg;
pub mod red_cast_elim;
pub mod type_infer;
pub mod util;

pub use call_site_flow::CallSiteTypeFlow;
pub use call_site_widen::CallSiteTypeWiden;
pub use cfg_simplify::CfgSimplify;
pub use const_fold::ConstantFolding;
pub use constraint_solve::ConstraintSolve;
pub use constraint_solve2::ConstraintSolve2;
pub use constructor_struct_infer::ConstructorStructInfer;
pub use coroutine_lower::CoroutineLowering;
pub use dce::DeadCodeElimination;
pub use int_to_bool::IntToBoolPromotion;
pub use mem2reg::Mem2Reg;
pub use red_cast_elim::RedundantCastElimination;
pub use type_infer::TypeInference;

use crate::pipeline::{PassConfig, PassDescriptor, TransformPipeline};

/// All known transform passes in the standard pipeline, in preferred
/// registration order.
///
/// Registration order determines execution order for passes without
/// `requires()` dependencies. Passes with `requires()` declarations are
/// topo-sorted after their dependencies.
///
/// Note: `IntToBoolPromotion` is frontend-injected via
/// `FrontendOutput::frontend_passes` and is NOT included here.
pub fn all_passes() -> Vec<PassDescriptor> {
    vec![
        PassDescriptor {
            name: "constructor-struct-infer",
            factory: || Box::new(ConstructorStructInfer),
            config_enabled: |c| c.constructor_struct_infer,
        },
        PassDescriptor {
            name: "type-inference",
            factory: || Box::new(TypeInference),
            config_enabled: |c| c.type_inference,
        },
        PassDescriptor {
            name: "call-site-type-flow",
            factory: || Box::new(CallSiteTypeFlow),
            config_enabled: |c| c.call_site_flow,
        },
        PassDescriptor {
            name: "constraint-solve",
            factory: || Box::new(ConstraintSolve),
            config_enabled: |c| c.constraint_solve,
        },
        PassDescriptor {
            name: "constraint-solve2",
            factory: || Box::new(ConstraintSolve2),
            config_enabled: |c| c.constraint_solve2,
        },
        PassDescriptor {
            name: "call-site-type-widen",
            factory: || Box::new(CallSiteTypeWiden),
            config_enabled: |c| c.call_site_widen,
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
    ]
}

/// Build a transform pipeline based on the given pass configuration.
///
/// Enabled passes are topo-sorted by their `requires()` declarations and
/// expanded by their `invalidates()` declarations, so re-runs (e.g. the
/// second TypeInference and ConstantFolding after Mem2Reg) are inserted
/// automatically.
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
    /// including automatic re-runs of TypeInference and ConstantFolding (after
    /// Mem2Reg) via invalidation expansion.
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
        let pos_ti: Vec<usize> = names
            .iter()
            .enumerate()
            .filter(|(_, &n)| n == "type-inference")
            .map(|(i, _)| i)
            .collect();
        let pos_cf: Vec<usize> = names
            .iter()
            .enumerate()
            .filter(|(_, &n)| n == "constant-folding")
            .map(|(i, _)| i)
            .collect();

        // TypeInference must appear twice:
        // (1) before Mem2Reg (the main inference run), and
        // (2) after Mem2Reg (which invalidates it).
        assert_eq!(
            pos_ti.len(),
            2,
            "expected 2 type-inference runs, got {:?}",
            pos_ti
        );
        assert!(
            pos_ti[0] < pos_mem2reg,
            "first type-inference must be before mem2reg"
        );
        assert!(
            pos_ti[1] > pos_mem2reg,
            "second type-inference must be after mem2reg"
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

        // RedundantCastElimination must appear after the last TypeInference and
        // last ConstantFolding.
        let pos_rce = names
            .iter()
            .position(|&n| n == "redundant-cast-elimination")
            .expect("redundant-cast-elimination missing");
        assert!(pos_rce > pos_ti[1], "rce must be after last type-inference");
        assert!(
            pos_rce > pos_cf[1],
            "rce must be after last constant-folding"
        );

        // DeadCodeElimination must be last.
        let pos_dce = names
            .iter()
            .position(|&n| n == "dead-code-elimination")
            .expect("dead-code-elimination missing");
        assert_eq!(
            pos_dce,
            names.len() - 1,
            "dead-code-elimination must be last"
        );

        // validate_ordering must not panic.
        pipeline.validate_ordering();
    }
}
