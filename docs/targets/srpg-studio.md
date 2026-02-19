# SRPG Studio

**Status: Planned** — No implementation started. Already JavaScript-based; web lift is relatively straightforward.

## Background

SRPG Studio is a tactical RPG creation tool by SystemSoft Alpha (publisher of the classic Dai Senryaku series), aimed at the Japanese market. Games use a Fire Emblem-like gameplay model with grid-based maps, unit management, and turn-based combat.

## Format

Published games ship two archive files:
- **`data.dts`** — custom encrypted archive containing all project data: maps, unit data, scripts, custom resources
- **`runtime.rts`** — unencrypted archive containing base engine resources: default graphics, audio, UI assets

The project format used during development is **`.srpgs`** (single project save file).

The archive format is proprietary and undocumented, but has been reverse-engineered:
- **[SRPG-ToolBox](https://github.com/Sinflower/SRPG-ToolBox)** — unpack/repack/translate `.dts` archives up to v1.317
- **[SRPG-Studio-extractor](https://github.com/godoway/SRPG-Studio-extractor)** — alternative extractor
- **[SRPG-Studio-asset-extractor](https://github.com/yiyuezhuo/SRPG-Studio-asset-extractor)**

## Runtime

**SRPG Studio runs on NW.js (Node.js + Chromium) as a desktop application.** The engine code is JavaScript. Game logic is written in JavaScript and executed in NW.js. The engine exposes a plugin API — developers extend behavior by writing `.js` plugin files. The base engine code is bundled JS (obfuscated in most distributions but not compiled to native code).

This makes SRPG Studio the most favorable engine on this list for a web lift: the engine is already JavaScript.

## Lifting Strategy

Tier 2 — strip NW.js-specific APIs, serve as a static web bundle.

1. Unpack `data.dts` (SRPG-ToolBox)
2. Extract engine JS and game JS/data
3. Replace NW.js-specific APIs with browser equivalents:
   - `nw.App.*` → stub or no-op
   - `nw.Window.*` → `window.*`
   - `fs.*` (Node file system) → fetch + IndexedDB
   - `path.*` → URL manipulation
   - `process.*` → stub
4. Bundle engine + game data as a static web app
5. Optionally: deobfuscate and annotate the engine JS for better maintainability

No IR pass is needed for a basic lift — the JS can run directly in a browser after API shim replacement.

## What Needs Building

### Extractor (new tool or crate: `reincarnate-frontend-srpgstudio`)

- [ ] `data.dts` decryption and extraction (adapt from SRPG-ToolBox)
- [ ] `runtime.rts` extraction (unencrypted, simpler)
- [ ] Asset catalog (images, audio, fonts referenced in engine data)

### NW.js → Browser Shim

- [ ] `nw.App` stub (application lifecycle, command line args)
- [ ] `nw.Window` stub (window management — not applicable in browser)
- [ ] Node.js `fs` → fetch/IndexedDB adapter
- [ ] Node.js `path` → URL helper stubs
- [ ] `process.platform` / `process.env` stubs
- [ ] `require()` shim (NW.js allows CommonJS require; browser needs bundling)

### Save / Load

SRPG Studio likely uses Node.js file I/O for save files. The shim maps:
- `fs.writeFileSync(savePath, data)` → `localStorage.setItem(key, data)` or IndexedDB
- `fs.readFileSync(savePath)` → `localStorage.getItem(key)` or IndexedDB

### Optional: Full Decompilation

For games where the plugin JS is complex, a full reincarnate pipeline could decompile the obfuscated game JS using an existing deobfuscator, then emit clean TypeScript. This is optional complexity for cases where simple API shimming is insufficient.

## Known Challenges

- **`data.dts` encryption versions** — SRPG-ToolBox supports up to v1.317; newer versions may use updated encryption
- **Obfuscation** — The base engine JS is obfuscated, making it harder to read and modify
- **NW.js privilege model** — Some features (e.g., local file dialogs, OS integration) have no browser equivalent; these need stubs or feature removal
- **WebGL vs Canvas** — SRPG Studio may use WebGL for rendering; this should already work in the browser but needs testing

## References

- [SRPG Studio official site](https://srpgstudio.com/)
- [SRPG-ToolBox](https://github.com/Sinflower/SRPG-ToolBox)
- [SRPG-Studio-extractor](https://github.com/godoway/SRPG-Studio-extractor)
