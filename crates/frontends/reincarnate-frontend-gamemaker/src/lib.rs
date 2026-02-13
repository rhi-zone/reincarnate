mod translate;

use std::collections::HashMap;
use std::fs;

use datawin::DataWin;
use reincarnate_core::error::CoreError;
use reincarnate_core::ir::builder::ModuleBuilder;
use reincarnate_core::pipeline::{Frontend, FrontendInput, FrontendOutput};
use reincarnate_core::project::{AssetCatalog, EngineOrigin};

use crate::translate::TranslateCtx;

/// GameMaker frontend — translates data.win files into reincarnate IR.
pub struct GameMakerFrontend;

impl Frontend for GameMakerFrontend {
    fn supported_engines(&self) -> &[EngineOrigin] {
        &[EngineOrigin::GameMaker]
    }

    fn extract(&self, input: FrontendInput) -> Result<FrontendOutput, CoreError> {
        let data = fs::read(&input.source)?;
        let dw = DataWin::parse(data).map_err(|e| CoreError::Parse {
            file: input.source.clone(),
            message: e.to_string(),
        })?;

        let parse_err = |e: datawin::Error| CoreError::Parse {
            file: input.source.clone(),
            message: e.to_string(),
        };

        let gen8 = dw.gen8().map_err(parse_err)?;
        let game_name = dw.resolve_string(gen8.name).map_err(|e| CoreError::Parse {
            file: input.source.clone(),
            message: format!("failed to resolve game name: {e}"),
        })?;

        eprintln!("[gamemaker] extracting: {game_name}");

        let code = dw.code().map_err(parse_err)?;
        let func = dw.func().map_err(parse_err)?;
        let scpt = dw.scpt().map_err(parse_err)?;
        let vari = dw.vari().map_err(parse_err)?;

        // Build function name lookup: function_id → resolved name.
        let function_names = build_function_names(&dw, func)?;

        // Build variable lookup: variable_id → (name, instance_type).
        let variables = build_variable_table(&dw, vari)?;

        // Build code_locals lookup: code entry name → CodeLocals.
        let code_locals_map = build_code_locals_map(&dw, func)?;

        let mut mb = ModuleBuilder::new(&game_name);

        // Translate scripts.
        let mut translated = 0;
        let mut errors = 0;
        for script in &scpt.scripts {
            let script_name = dw.resolve_string(script.name).map_err(|e| CoreError::Parse {
                file: input.source.clone(),
                message: format!("failed to resolve script name: {e}"),
            })?;

            let code_idx = script.code_id as usize;
            if code_idx >= code.entries.len() {
                eprintln!("[gamemaker] warn: script {script_name} references invalid code entry {code_idx}");
                continue;
            }

            let bytecode = match code.entry_bytecode(code_idx, dw.data()) {
                Some(bc) => bc,
                None => {
                    eprintln!("[gamemaker] warn: no bytecode for script {script_name}");
                    continue;
                }
            };

            let code_entry = &code.entries[code_idx];
            let code_name = dw.resolve_string(code_entry.name).unwrap_or_default();
            let clean_name = strip_script_prefix(&script_name);

            let locals = code_locals_map.get(&code_name).copied();

            let ctx = TranslateCtx {
                dw: &dw,
                function_names: &function_names,
                variables: &variables,
                locals,
                has_self: false,
                has_other: false,
                arg_count: code_entry.args_count & 0x7FFF, // mask off weird local flag
            };

            match translate::translate_code_entry(bytecode, clean_name, &ctx) {
                Ok(func) => {
                    mb.add_function(func);
                    translated += 1;
                }
                Err(e) => {
                    eprintln!("[gamemaker] error translating {clean_name}: {e}");
                    errors += 1;
                }
            }
        }

        eprintln!("[gamemaker] translated {translated} scripts ({errors} errors)");

        let module = mb.build();

        Ok(FrontendOutput {
            modules: vec![module],
            assets: AssetCatalog::default(),
        })
    }
}

/// Build function_id → resolved name mapping from FUNC entries.
fn build_function_names(
    dw: &DataWin,
    func: &datawin::chunks::func::Func,
) -> Result<HashMap<u32, String>, CoreError> {
    let mut names = HashMap::new();
    for (idx, entry) in func.functions.iter().enumerate() {
        let name = dw.resolve_string(entry.name).unwrap_or_else(|_| format!("func_{idx}"));
        names.insert(idx as u32, name);
    }
    Ok(names)
}

/// Build variable_id → (name, instance_type) from VARI entries.
fn build_variable_table(
    dw: &DataWin,
    vari: &datawin::chunks::vari::Vari,
) -> Result<Vec<(String, i32)>, CoreError> {
    let mut vars = Vec::with_capacity(vari.variables.len());
    for entry in &vari.variables {
        let name = dw.resolve_string(entry.name).unwrap_or_else(|_| "???".to_string());
        vars.push((name, entry.instance_type));
    }
    Ok(vars)
}

/// Build code entry name → CodeLocals mapping.
fn build_code_locals_map<'a>(
    dw: &DataWin,
    func: &'a datawin::chunks::func::Func,
) -> Result<HashMap<String, &'a datawin::chunks::func::CodeLocals>, CoreError> {
    let mut map = HashMap::new();
    for entry in &func.code_locals {
        let name = dw.resolve_string(entry.name).unwrap_or_default();
        map.insert(name, entry);
    }
    Ok(map)
}

/// Strip common GML script prefixes to get a clean function name.
fn strip_script_prefix(name: &str) -> &str {
    name.strip_prefix("gml_GlobalScript_")
        .or_else(|| name.strip_prefix("gml_Script_"))
        .unwrap_or(name)
}
