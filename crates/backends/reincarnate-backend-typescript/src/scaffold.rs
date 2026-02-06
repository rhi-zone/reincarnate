//! Generate project scaffold files: index.html, tsconfig.json, and main entry point.

use std::fmt::Write;
use std::fs;
use std::path::Path;

use reincarnate_core::error::CoreError;
use reincarnate_core::ir::{Module, Visibility};

/// Write all scaffold files into `output_dir`.
pub fn emit_scaffold(modules: &[Module], output_dir: &Path) -> Result<(), CoreError> {
    fs::write(
        output_dir.join("index.html"),
        generate_index_html(modules),
    )?;
    fs::write(output_dir.join("tsconfig.json"), TSCONFIG)?;
    fs::write(output_dir.join("main.ts"), generate_main(modules))?;
    Ok(())
}

fn generate_index_html(modules: &[Module]) -> String {
    let title = modules
        .first()
        .map(|m| m.name.as_str())
        .unwrap_or("Reincarnate App");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{title}</title>
  <style>
    body {{
      margin: 0;
      background: #000;
      display: flex;
      justify-content: center;
      align-items: center;
      height: 100vh;
    }}
    canvas {{
      image-rendering: pixelated;
    }}
  </style>
</head>
<body>
  <canvas id="reincarnate-canvas" width="800" height="600"></canvas>
  <script type="module" src="./main.ts"></script>
</body>
</html>
"#
    )
}

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ES2020",
    "moduleResolution": "bundler",
    "strict": true,
    "esModuleInterop": true,
    "outDir": "dist",
    "rootDir": ".",
    "lib": ["ES2020", "DOM", "DOM.Iterable"]
  },
  "include": ["*.ts", "runtime/*.ts"]
}
"#;

/// Generate `main.ts` — imports all modules and calls the entry point in a
/// requestAnimationFrame game loop.
fn generate_main(modules: &[Module]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "import {{ timing }} from \"./runtime\";");

    // Import all public functions from each module.
    let mut entry_func: Option<(String, String)> = None;
    for module in modules {
        let public_names: Vec<&str> = module
            .functions
            .values()
            .filter(|f| f.visibility == Visibility::Public)
            .map(|f| f.name.as_str())
            .collect();
        if public_names.is_empty() {
            continue;
        }
        let _ = writeln!(
            out,
            "import {{ {} }} from \"./{}\";",
            public_names.join(", "),
            module.name,
        );
        // Pick the first plausible entry point.
        if entry_func.is_none() {
            for &name in &public_names {
                if is_entry_candidate(name) {
                    entry_func = Some((module.name.clone(), name.to_string()));
                    break;
                }
            }
        }
    }

    out.push('\n');

    match entry_func {
        Some((_module, func_name)) => {
            let _ = writeln!(out, "function loop() {{");
            let _ = writeln!(out, "  timing.tick();");
            let _ = writeln!(out, "  {func_name}();");
            let _ = writeln!(out, "  requestAnimationFrame(loop);");
            let _ = writeln!(out, "}}");
            let _ = writeln!(out);
            let _ = writeln!(out, "requestAnimationFrame(loop);");
        }
        None => {
            // No obvious entry point — just import everything and let the
            // module top-level code run.
            let _ = writeln!(out, "// No entry point detected. Module-level code will run on import.");
        }
    }

    out
}

fn is_entry_candidate(name: &str) -> bool {
    matches!(
        name,
        "main"
            | "init"
            | "start"
            | "run"
            | "update"
            | "tick"
            | "frame"
            | "enterFrame"
            | "onEnterFrame"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reincarnate_core::ir::builder::{FunctionBuilder, ModuleBuilder};
    use reincarnate_core::ir::{FunctionSig, Type};

    #[test]
    fn main_with_entry_point() {
        let mut mb = ModuleBuilder::new("game");
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
        };
        let mut fb = FunctionBuilder::new("update", sig.clone(), Visibility::Public);
        fb.ret(None);
        mb.add_function(fb.build());
        let mut fb2 = FunctionBuilder::new("helper", sig, Visibility::Private);
        fb2.ret(None);
        mb.add_function(fb2.build());
        let module = mb.build();

        let main = generate_main(&[module]);
        assert!(main.contains("import { update } from \"./game\";"));
        assert!(main.contains("update();"));
        assert!(main.contains("requestAnimationFrame(loop);"));
        // Private function not imported.
        assert!(!main.contains("helper"));
    }

    #[test]
    fn main_no_entry_point() {
        let mut mb = ModuleBuilder::new("utils");
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
        };
        let mut fb = FunctionBuilder::new("compute", sig, Visibility::Public);
        fb.ret(None);
        mb.add_function(fb.build());
        let module = mb.build();

        let main = generate_main(&[module]);
        assert!(main.contains("import { compute } from \"./utils\";"));
        assert!(main.contains("No entry point detected"));
        assert!(!main.contains("requestAnimationFrame"));
    }

    #[test]
    fn index_html_has_canvas() {
        let mb = ModuleBuilder::new("my_game");
        let module = mb.build();

        let html = generate_index_html(&[module]);
        assert!(html.contains("reincarnate-canvas"));
        assert!(html.contains("<title>my_game</title>"));
        assert!(html.contains("main.ts"));
    }
}
