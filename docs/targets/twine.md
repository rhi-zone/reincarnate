# Twine

Twine games are distributed as self-contained HTML files. The game engine and all passage content are embedded in the HTML. Reincarnate supports two Twine story formats:

| Format | Status | Notes |
|--------|--------|-------|
| [SugarCube](./sugarcube) | ✅ Active | Dominant format for large/complex games |
| [Harlowe](./harlowe) | ✅ Active | Twine 2 default; high-value lifting target |

Two other formats exist but are not planned:

- **Snowman** — Passages contain raw JavaScript with `<% %>` template tags. There is no macro DSL to lift; the source is already the target.
- **Chapbook** — Minimal adoption, niche syntax, no significant ecosystem.

## Shared Infrastructure

Both formats use the same HTML extraction pipeline (html5ever tokenizer), the same IR, transform passes, and TypeScript backend. They differ in:

- Frontend parser (`sugarcube/` vs `harlowe/` modules)
- Replacement runtime (`runtime/twine/ts/` — separate namespaces)
- Runtime config (`runtime.json` for SugarCube, `runtime.harlowe.json` for Harlowe)
- SystemCall namespaces: `SugarCube.*` vs `Harlowe.{State,Output,Navigation,Engine}`
- Scaffold HTML structure (`<div id="passages">` vs `<tw-story>`)
