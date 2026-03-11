pub mod backend;
pub mod checker;
pub mod config;
pub mod frontend;
pub mod linker;
pub mod transform;

pub use backend::{Backend, BackendInput, BackendOutput, RuntimePackage};
pub use checker::{CheckSummary, Checker, CheckerInput, CheckerOutput, Diagnostic, Severity};
pub use config::{resolve_preset, DebugConfig, LoweringConfig, PassConfig};
pub use frontend::{Frontend, FrontendInput, FrontendOutput};
pub use linker::{link_modules, SymbolTable};
pub use transform::{
    PipelineOutput, Transform, TransformPipeline, TransformResult, VALID_PASS_NAMES,
};
