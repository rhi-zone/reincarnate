# Ink (by Inkle Studios)

**Status: Planned** — No implementation started. Web runtime already exists; primary value is extraction from host games.

## Format

- **`.ink`** — plain-text source in the Ink markup language (knots, stitches, choices, conditional logic, variables, functions)
- **`.json`** — compiled runtime format produced by `inklecate`. This is the distribution artifact and what the runtime consumes.

The JSON format is **fully documented** in [ink_JSON_runtime_format.md](https://github.com/inkle/ink/blob/master/Documentation/ink_JSON_runtime_format.md). It represents a tree of **Containers** (JSON arrays). Array elements are: prefixed string content (`^text`), control commands (`"ev"`, `"/ev"`, `"out"`, `"pop"`, `"->->"`, `"~ret"`), diverts (jumps/calls), integers, floats, and variable references. Paths use dot-separated notation (`knot.stitch.0`). The last element of each container is either `null` or an object with named sub-containers and metadata flags.

## Runtime

Stack-based VM. Steps through container elements maintaining:
- An output buffer (text and tags)
- An evaluation stack
- Variable state (story variables + temporary variables)
- Visit counts per container (built-in — `"visit"` opcode)
- Divert targets (knots, stitches, tunnels, threads)

Control commands manipulate the stack. Diverts implement function calls, tunnels (subroutines with `->->` return), and thread cloning. Choice points are collected during traversal and presented to the player. The runtime is intentionally non-prescriptive about UI.

## Existing Web Runtime

**[inkle/inkjs](https://github.com/inkle/inkjs)** is the official JavaScript port of the ink engine (also on npm as `inkjs`). It is:
- Fully compatible with `.json` story files
- Zero dependencies, runs in all browsers and Node.js
- As of v2.1.0, includes a compiled-in ink compiler (`.ink` → parse → play in JS)
- The runtime used by Inky (the official editor) for preview
- Maintained by inkle (MIT license)

A reincarnate ink frontend would not need to reimplement the runtime — inkjs already handles execution.

## Lifting Strategy

The primary use case for reincarnate is **extracting ink stories from host games** (e.g., Unity games or native apps that embed ink) and re-hosting them with the web runtime.

### Extraction

Many commercial games bundle ink as `.json` files in their game data directory. The extraction path is:
1. Locate `.json` ink files in the game package (Unity games: in `Assets/` or `StreamingAssets/`)
2. Validate against the ink JSON schema
3. Generate a host HTML + inkjs entry point

For games that compile ink into a native runtime (the C# ink-engine-runtime is embedded in Unity builds), the `.json` files may not be present. These require binary extraction.

### Optional: Transpile to IR

For games where deeper integration is needed (e.g., mixing ink narrative with GameMaker or Flash code), a reincarnate ink frontend could:
1. Parse the `.json` container format
2. Emit IR — each knot/stitch becomes a function, choices become `SystemCall("Ink.Choice", ...)` + `Yield`
3. Emit TypeScript using the existing backend

This is optional complexity — inkjs already handles execution correctly.

## What Needs Building (if full frontend is desired)

- [ ] `.ink` source parser (for games with source present)
- [ ] `.json` reader → IR emitter (containers → functions, diverts → branches/calls, choices → yield points)
- [ ] `SystemCall` namespace: `Ink.Output`, `Ink.Choice`, `Ink.Tag`
- [ ] Replacement runtime (thin wrapper around inkjs, or direct TypeScript port)

## References

- [ink source (MIT)](https://github.com/inkle/ink)
- [inkjs (official JS port)](https://github.com/inkle/inkjs)
- [ink JSON runtime format documentation](https://github.com/inkle/ink/blob/master/Documentation/ink_JSON_runtime_format.md)
- [Inky editor](https://github.com/inkle/inky)
