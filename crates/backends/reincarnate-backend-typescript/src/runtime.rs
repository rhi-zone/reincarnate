use std::fs;
use std::path::Path;

use reincarnate_core::error::CoreError;

const RENDERER_TS: &str = include_str!("../runtime/renderer.ts");
const AUDIO_TS: &str = include_str!("../runtime/audio.ts");
const INPUT_TS: &str = include_str!("../runtime/input.ts");
const TIMING_TS: &str = include_str!("../runtime/timing.ts");
const SAVE_TS: &str = include_str!("../runtime/save.ts");
const UI_TS: &str = include_str!("../runtime/ui.ts");
const INDEX_TS: &str = include_str!("../runtime/index.ts");

/// All known system names that the runtime provides.
pub const SYSTEM_NAMES: &[&str] = &["renderer", "audio", "input", "timing", "save", "ui"];

/// Write the runtime TypeScript files into `output_dir/runtime/`.
pub fn emit_runtime(output_dir: &Path) -> Result<(), CoreError> {
    let runtime_dir = output_dir.join("runtime");
    fs::create_dir_all(&runtime_dir)?;

    fs::write(runtime_dir.join("renderer.ts"), RENDERER_TS)?;
    fs::write(runtime_dir.join("audio.ts"), AUDIO_TS)?;
    fs::write(runtime_dir.join("input.ts"), INPUT_TS)?;
    fs::write(runtime_dir.join("timing.ts"), TIMING_TS)?;
    fs::write(runtime_dir.join("save.ts"), SAVE_TS)?;
    fs::write(runtime_dir.join("ui.ts"), UI_TS)?;
    fs::write(runtime_dir.join("index.ts"), INDEX_TS)?;

    Ok(())
}
