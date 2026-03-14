#!/usr/bin/env bun
/**
 * gml-manual-sigs.ts — Extract GML function signatures from the GameMaker Manual.
 *
 * Clones https://github.com/YoYoGames/GameMaker-Manual (if not already present),
 * parses the HTML function pages, and outputs a JSON object mapping function names
 * to { params: string[], returns: string } — the same shape as runtime.json's
 * function_signatures section.
 *
 * Usage:
 *   bun scripts/gml-manual-sigs.ts                    # extract all sigs
 *   bun scripts/gml-manual-sigs.ts --diff             # compare against runtime.json
 *   bun scripts/gml-manual-sigs.ts --func draw_text   # extract one function
 */

import { existsSync } from "fs";
import { readdir, readFile } from "fs/promises";
import { join, basename } from "path";
import { execSync } from "child_process";

const MANUAL_DIR = join(import.meta.dir, "..", ".gml-manual");
const MANUAL_REPO = "https://github.com/YoYoGames/GameMaker-Manual.git";
const GML_REF_DIR = join(
  MANUAL_DIR,
  "Manual/contents/GameMaker_Language/GML_Reference"
);
const RUNTIME_JSON = join(
  import.meta.dir,
  "..",
  "runtime/gamemaker/ts/runtime.json"
);

// -- Type mapping from GML manual data-keyref values to runtime.json types --

const TYPE_MAP: Record<string, string> = {
  Type_Real: "number",
  Type_Bool: "boolean",
  Type_String: "string",
  Type_Void: "void",
  Type_Undefined: "void",
  Type_Any: "any", // TODO: should not exist in output — flag for review
  Type_Array: "any[]",

  // Asset/resource types — all numeric indices at runtime
  Type_Asset_Sprite: "number",
  Type_Asset_Tileset: "number",
  Type_Asset_Tile_Set: "number",
  Type_Asset_Sound: "number",
  Type_Asset_Font: "number",
  Type_Asset_Path: "number",
  Type_Asset_Object: "number",
  Type_Asset_Room: "number",
  Type_Asset_Script: "number",
  Type_Asset_Timeline: "number",
  Type_Asset_Sequence: "number",
  Type_Asset_AnimCurve: "number",
  Type_Asset_Shader: "number",
  Type_Asset_ParticleSystem: "number",

  // ID types — all numeric at runtime
  Type_ID_Instance: "number",
  Type_ID_Element_Tilemap: "number",
  Type_ID_Layer: "number",
  Type_ID_Camera: "number",
  Type_ID_Surface: "number",
  Type_ID_Buffer: "number",
  Type_ID_Sound_Instance: "number",
  Type_ID_Particle_System: "number",
  Type_ID_Particle_Type: "number",
  Type_ID_Particle_Emitter: "number",
  Type_ID_DS_Map: "number",
  Type_ID_DS_List: "number",
  Type_ID_DS_Grid: "number",
  Type_ID_DS_Stack: "number",
  Type_ID_DS_Queue: "number",
  Type_ID_DS_Priority: "number",
  Type_ID_Vertex_Buffer: "number",
  Type_ID_Vertex_Format: "number",
  Type_ID_TimeSource: "number",
  Type_ID_AudioEmitter: "number",
  Type_ID_AudioListener: "number",
  Type_ID_AudioGroup: "number",
  Type_ID_AudioBus: "number",
  Type_ID_AudioEffect: "number",

  // Struct types
  Type_Struct: "any", // TODO: could be more specific
  Type_Struct_WeakRef: "any",
  Type_Struct_Font_Info: "any",

  // Function/method types — all opaque at runtime
  Type_Function: "any",
  Type_Function_Script: "any",
  Type_Method: "any",

  // Pointer types — numeric handles
  Type_Pointer: "number",
  Type_Pointer_Texture: "number",
  Type_Handle_Shader_Uniform: "number",
  Type_Handle_Shader_Sampler: "number",

  // Misc numeric types
  Type_Real_Datetime: "number",
  Type_Real_int64: "number",
  Type_Real_Tile_Data: "number",
  Type_Array_Matrix: "any[]",
  Type_Asset: "number",
  Type_Constant: "number",
  Type_Enum_ColSpace: "number",
  Collision_obj_type: "number",
};

function resolveType(keyref: string): string {
  if (TYPE_MAP[keyref]) return TYPE_MAP[keyref];
  // Fallback: if starts with Type_ID_ or Type_Asset_, assume number
  if (keyref.startsWith("Type_ID_") || keyref.startsWith("Type_Asset_"))
    return "number";
  // Constant types (enums) — numeric
  if (keyref.startsWith("Type_Constant_") || keyref.startsWith("Constant_"))
    return "number";
  return `UNKNOWN(${keyref})`;
}

interface FuncSig {
  params: string[];
  returns: string;
  optional_from?: number; // index of first optional param
}

function parseFunctionPage(html: string, filename: string): FuncSig | null {
  const funcName = basename(filename, ".htm");

  // Find syntax block
  const syntaxMatch = html.match(
    /<h4>Syntax:<\/h4>\s*<p class="code">([\s\S]*?)<\/p>/
  );
  if (!syntaxMatch) return null;

  // Extract param names from syntax to detect optional params (wrapped in [...])
  const syntaxText = syntaxMatch[1].replace(/<[^>]*>/g, "").trim();
  const paramMatch = syntaxText.match(/\(([^)]*)\)/);
  if (!paramMatch && !syntaxText.includes("(")) return null;

  const syntaxParams = paramMatch
    ? paramMatch[1]
        .split(",")
        .map((p) => p.trim())
        .filter((p) => p.length > 0)
    : [];
  const optionalNames = new Set(
    syntaxParams
      .filter((p) => p.startsWith("["))
      .map((p) => p.replace(/[\[\]]/g, "").trim())
  );

  // Parse parameter table
  const tableMatch = html.match(
    /<h4>Syntax:<\/h4>[\s\S]*?<table>([\s\S]*?)<\/table>/
  );
  const params: string[] = [];
  let optionalFrom: number | undefined;

  if (tableMatch) {
    const rows = tableMatch[1].match(/<tr>([\s\S]*?)<\/tr>/g) || [];
    // Skip header row
    for (let i = 1; i < rows.length; i++) {
      const cells = rows[i].match(/<td>([\s\S]*?)<\/td>/g) || [];
      if (cells.length < 2) continue;

      const paramName = cells[0].replace(/<[^>]*>/g, "").trim();
      const typeCell = cells[1];

      // Extract data-keyref
      const keyrefMatch = typeCell.match(/data-keyref="([^"]+)"/);
      let type = "any";
      if (keyrefMatch) {
        type = resolveType(keyrefMatch[1]);
      } else {
        // Fallback: check text content
        const text = typeCell.replace(/<[^>]*>/g, "").trim();
        if (text === "Real" || text === "Number") type = "number";
        else if (text === "String") type = "string";
        else if (text === "Boolean" || text === "Bool") type = "boolean";
      }

      params.push(type);

      if (optionalNames.has(paramName) && optionalFrom === undefined) {
        optionalFrom = i - 1;
      }
    }
  }

  // Parse return type
  const returnsMatch = html.match(
    /<h4>Returns:<\/h4>\s*<p class="code">([\s\S]*?)<\/p>/
  );
  let returns = "void";
  if (returnsMatch) {
    const keyrefMatch = returnsMatch[1].match(/data-keyref="([^"]+)"/);
    if (keyrefMatch) {
      returns = resolveType(keyrefMatch[1]);
    } else {
      const text = returnsMatch[1].replace(/<[^>]*>/g, "").trim();
      if (text === "N/A") returns = "void";
      else if (text === "Real" || text === "Number") returns = "number";
      else if (text === "String") returns = "string";
      else if (text === "Boolean" || text === "Bool") returns = "boolean";
    }
  }

  return { params, returns, ...(optionalFrom !== undefined && { optional_from: optionalFrom }) };
}

async function walkDir(dir: string): Promise<string[]> {
  const results: string[] = [];
  const entries = await readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      results.push(...(await walkDir(full)));
    } else if (entry.name.endsWith(".htm")) {
      results.push(full);
    }
  }
  return results;
}

async function main() {
  const args = process.argv.slice(2);
  const diffMode = args.includes("--diff");
  const funcFilter = args.includes("--func")
    ? args[args.indexOf("--func") + 1]
    : null;

  // Clone if needed
  if (!existsSync(MANUAL_DIR)) {
    console.error(`Cloning GameMaker Manual to ${MANUAL_DIR}...`);
    execSync(`git clone --depth 1 ${MANUAL_REPO} ${MANUAL_DIR}`, {
      stdio: "inherit",
    });
  }

  if (!existsSync(GML_REF_DIR)) {
    console.error(`ERROR: GML Reference dir not found at ${GML_REF_DIR}`);
    process.exit(1);
  }

  // Parse all function pages
  const files = await walkDir(GML_REF_DIR);
  const sigs: Record<string, FuncSig> = {};
  let parsed = 0;
  let skipped = 0;

  for (const file of files) {
    const name = basename(file, ".htm");

    // Skip index/category pages (capitalized or contain spaces)
    if (name.includes(" ") || name[0] === name[0].toUpperCase()) {
      skipped++;
      continue;
    }

    if (funcFilter && name !== funcFilter) continue;

    const html = await readFile(file, "utf-8");
    const sig = parseFunctionPage(html, file);
    if (sig) {
      sigs[name] = sig;
      parsed++;
    } else {
      skipped++;
    }
  }

  if (!diffMode && !funcFilter) {
    // Output all signatures
    const output: Record<string, { params: string[]; returns: string }> = {};
    for (const [name, sig] of Object.entries(sigs).sort(([a], [b]) =>
      a.localeCompare(b)
    )) {
      output[name] = { params: sig.params, returns: sig.returns };
    }
    console.log(JSON.stringify(output, null, 2));
    console.error(`\nParsed ${parsed} functions, skipped ${skipped} files`);
  } else if (funcFilter) {
    const sig = sigs[funcFilter];
    if (sig) {
      console.log(JSON.stringify({ [funcFilter]: sig }, null, 2));
    } else {
      console.error(`Function '${funcFilter}' not found in manual`);
      process.exit(1);
    }
  } else {
    // Diff mode: compare against runtime.json
    const runtimeJson = JSON.parse(await readFile(RUNTIME_JSON, "utf-8"));
    const currentSigs = runtimeJson.function_signatures || {};

    // Normalize a type for comparison
    // - int and number are both GML Real
    // - classref is a numeric object index at runtime
    // - * and dynamic are our names for "any"
    const normalize = (t: string): string => {
      if (t === "int") return "number";
      if (t === "classref") return "number";
      if (t === "*" || t === "dynamic") return "any";
      return t;
    };

    const typesEquiv = (a: string, b: string): boolean =>
      normalize(a) === normalize(b);

    let exactMatches = 0;
    let equivMatches = 0; // match after int/number normalization
    let arityMismatches = 0;
    let typeMismatches = 0;
    let anyInRuntime = 0; // runtime.json has `any` where manual has a concrete type
    let missing = 0;
    let extra = 0;

    const arityErrors: string[] = [];
    const typeErrors: string[] = [];
    const anyErrors: string[] = [];

    for (const [name, current] of Object.entries(currentSigs) as [
      string,
      any
    ][]) {
      const manual = sigs[name];
      if (!manual) {
        extra++;
        continue;
      }

      const currentParams: string[] = current.params || [];
      const manualParams = manual.params;
      const currentReturns: string = current.returns || "void";
      const manualReturns = manual.returns;

      // Check for optional params — if manual has optional_from, runtime may
      // have fewer params (only the required ones)
      const requiredManualCount = manual.optional_from ?? manualParams.length;
      const arityOk =
        currentParams.length === manualParams.length ||
        (manual.optional_from !== undefined &&
          currentParams.length >= requiredManualCount &&
          currentParams.length <= manualParams.length);

      if (!arityOk) {
        arityMismatches++;
        arityErrors.push(
          `ARITY: ${name}` +
            `\n  runtime.json: (${currentParams.join(", ")}) → ${currentReturns}  [${currentParams.length} params]` +
            `\n  manual:       (${manualParams.join(", ")}) → ${manualReturns}  [${manualParams.length} params${manual.optional_from !== undefined ? `, optional from ${manual.optional_from}` : ""}]`
        );
        continue;
      }

      // Check types (with normalization)
      let exact = true;
      let equiv = true;
      let hasAnyMismatch = false;
      const compareLen = Math.min(currentParams.length, manualParams.length);

      for (let i = 0; i < compareLen; i++) {
        if (currentParams[i] !== manualParams[i]) exact = false;
        if (!typesEquiv(currentParams[i], manualParams[i])) {
          equiv = false;
          // Check if runtime has `any` where manual has a concrete type
          if (currentParams[i] === "any" && manualParams[i] !== "any") {
            hasAnyMismatch = true;
          }
        }
      }
      if (currentReturns !== manualReturns) exact = false;
      if (!typesEquiv(currentReturns, manualReturns)) {
        equiv = false;
        if (currentReturns === "any" && manualReturns !== "any") {
          hasAnyMismatch = true;
        }
      }

      if (exact) {
        exactMatches++;
      } else if (equiv) {
        equivMatches++;
      } else if (hasAnyMismatch) {
        anyInRuntime++;
        anyErrors.push(
          `ANY: ${name}` +
            `\n  runtime.json: (${currentParams.join(", ")}) → ${currentReturns}` +
            `\n  manual:       (${manualParams.join(", ")}) → ${manualReturns}`
        );
      } else {
        typeMismatches++;
        typeErrors.push(
          `TYPE: ${name}` +
            `\n  runtime.json: (${currentParams.join(", ")}) → ${currentReturns}` +
            `\n  manual:       (${manualParams.join(", ")}) → ${manualReturns}`
        );
      }
    }

    // Check for functions in manual but not in runtime.json
    for (const name of Object.keys(sigs)) {
      if (!currentSigs[name]) {
        missing++;
      }
    }

    // Print grouped output
    if (arityErrors.length > 0) {
      console.log("=== ARITY MISMATCHES (wrong param count) ===\n");
      arityErrors.forEach((e) => console.log(e));
      console.log();
    }
    if (typeErrors.length > 0) {
      console.log("=== TYPE MISMATCHES (same arity, different types) ===\n");
      typeErrors.forEach((e) => console.log(e));
      console.log();
    }
    if (anyErrors.length > 0) {
      console.log(
        "=== ANY IN RUNTIME (runtime.json uses `any` where manual has a concrete type) ===\n"
      );
      anyErrors.forEach((e) => console.log(e));
      console.log();
    }

    console.log(`--- Summary ---`);
    console.log(`Exact matches:      ${exactMatches}`);
    console.log(`Equiv matches:      ${equivMatches} (int≡number)`);
    console.log(`Arity mismatches:   ${arityMismatches}`);
    console.log(`Type mismatches:    ${typeMismatches}`);
    console.log(`\`any\` in runtime:   ${anyInRuntime}`);
    console.log(
      `In runtime.json but not manual: ${extra} (platform/DnD/custom)`
    );
    console.log(`In manual but not runtime.json: ${missing}`);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
