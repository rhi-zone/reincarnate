pub mod const_fold;
pub mod type_infer;

pub use const_fold::ConstantFolding;
pub use type_infer::TypeInference;

use crate::pipeline::{PassConfig, TransformPipeline};

/// Build a transform pipeline based on the given pass configuration.
pub fn default_pipeline(config: &PassConfig) -> TransformPipeline {
    let mut pipeline = TransformPipeline::new();
    if config.type_inference {
        pipeline.add(Box::new(TypeInference));
    }
    if config.constant_folding {
        pipeline.add(Box::new(ConstantFolding));
    }
    pipeline
}
