use std::path::PathBuf;

use crate::error::CoreError;
use crate::ir::Module;
use crate::pipeline::Transform;
use crate::project::{AssetCatalog, EngineOrigin};

/// Input to a frontend.
pub struct FrontendInput {
    /// Path to the source binary/project.
    pub source: PathBuf,
    /// Engine origin hint (from manifest).
    pub engine: EngineOrigin,
    /// Frontend-specific options from the project manifest.
    pub options: serde_json::Value,
}

/// Output from a frontend.
pub struct FrontendOutput {
    /// The IR modules extracted from the source.
    pub modules: Vec<Module>,
    /// Assets extracted alongside the code.
    pub assets: AssetCatalog,
    /// Optional variant hint for runtime config selection.
    ///
    /// When set, the CLI loads `runtime.{variant}.json` instead of the
    /// default `runtime.json`. This lets a single engine (e.g. Twine) use
    /// different runtime configurations for different sub-formats
    /// (e.g. SugarCube vs Harlowe).
    pub runtime_variant: Option<String>,
    /// Engine-specific IR transform passes to run after the standard pipeline.
    ///
    /// These run after DCE (the last standard pass) but before structurization.
    /// Use this to inject engine-specific IR normalizations that the shared
    /// pipeline doesn't know about (e.g. GML logical-op pattern restoration).
    pub extra_passes: Vec<Box<dyn Transform>>,
}

/// Frontend trait — parses engine-specific formats and emits IR.
pub trait Frontend {
    /// Which engine(s) this frontend supports.
    fn supported_engines(&self) -> &[EngineOrigin];

    /// Parse the source and produce IR modules + extracted assets.
    fn extract(&self, input: FrontendInput) -> Result<FrontendOutput, CoreError>;
}
