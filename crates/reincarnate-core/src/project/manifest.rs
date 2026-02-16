use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

/// The source engine a project was extracted from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineOrigin {
    Flash,
    Director,
    Vb6,
    JavaApplet,
    Silverlight,
    HyperCard,
    ToolBook,
    RenPy,
    RpgMakerVxAce,
    RpgMakerMv,
    RpgMakerMz,
    GameMaker,
    Twine,
    Inform,
    Other(String),
}

/// Codegen backend target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetBackend {
    Rust,
    TypeScript,
}

/// Configuration for a build target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetConfig {
    pub backend: TargetBackend,
    pub output_dir: PathBuf,
    /// Additional backend-specific options.
    #[serde(default)]
    pub options: serde_json::Value,
}

/// An asset entry in the manifest: either a plain path (copied as-is) or
/// a `{ "src": "...", "dest": "..." }` object for path remapping.
#[derive(Debug, Clone, Serialize)]
pub struct AssetMapping {
    /// Source path (relative to manifest, resolved to absolute by CLI).
    pub src: PathBuf,
    /// Output path (relative to the output directory). If `None`, uses the
    /// source's filename/dirname.
    pub dest: Option<PathBuf>,
}

impl AssetMapping {
    /// The output-relative path for this asset.
    pub fn dest_name(&self) -> &std::path::Path {
        self.dest
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new(self.src.file_name().unwrap_or_default()))
    }
}

impl<'de> Deserialize<'de> for AssetMapping {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Plain(PathBuf),
            Mapped { src: PathBuf, dest: Option<PathBuf> },
        }
        match Raw::deserialize(deserializer)? {
            Raw::Plain(src) => Ok(AssetMapping { src, dest: None }),
            Raw::Mapped { src, dest } => Ok(AssetMapping { src, dest }),
        }
    }
}

/// Top-level project manifest (reincarnate.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub name: String,
    pub version: String,
    pub engine: EngineOrigin,
    /// Path to the source binary/project.
    pub source: PathBuf,
    pub targets: Vec<TargetConfig>,
    /// Asset directories/files to copy into the build output.
    ///
    /// Each entry is either a plain string path (copied preserving its name)
    /// or `{ "src": "...", "dest": "..." }` for path remapping.
    #[serde(default)]
    pub assets: Vec<AssetMapping>,
}
