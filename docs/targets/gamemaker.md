# GameMaker (GML)

**Status: Active** — GMS1 and GMS2/GMS2.3+ frontends are functional. Test projects: Bounty (GMS1) and Dead Estate (GMS2.3+, 0 errors).

## Format

GameMaker games ship as `data.win` files (Windows), with platform variants for other targets. The format contains:
- **GML bytecode** — a stack-based VM with typed operands
- **Asset tables** — sprites (SPRT), sounds (SOND), objects (OBJT), rooms (ROOM), fonts (FONT), scripts (SCPT), and more
- **String pool** — all strings referenced by bytecode
- **Texture pages** — atlas images for sprites/backgrounds/fonts

The format is partially documented through community reverse-engineering (UndertaleModTool, GameMaker Data Reader). GMS2.3+ introduced significant changes (anonymous functions as closures, structs, sequences, animation curves).

## Lifting Strategy

Full recompilation. The `reincarnate-frontend-gamemaker` crate (backed by the `datawin` crate for format parsing):
1. Parses `data.win` using the `datawin` crate
2. Decodes GML bytecode per object/event/script
3. Emits IR per function, with instance variable access modeled as field reads/writes
4. Identifies `GameMaker.*` system call boundaries

The replacement runtime in `runtime/gamemaker/ts/` implements GML object instances, room management, drawing, input, and audio stubs.

## Implementation Status

### Frontend completeness — GMS1

- ✅ All standard opcodes (push, pop, call, branch, comparison, arithmetic)
- ✅ VARI scope dispatch (self, global, local, builtin, stacked instances)
- ✅ 2D array access (ref_type==0 with instance field as scope owner, not target)
- ✅ Object event handlers (Create, Destroy, Step, Draw, Alarm, Keyboard, Mouse, Collision, User)
- ✅ `argument` variable mapping to function parameters
- ✅ `hasNext2`/`pushenvv`/`popenvv` (for-in loop lowering)
- ✅ Object inheritance via `event_inherited`
- ✅ Object properties (persistent, visible) emitted as class fields when non-default

### Frontend completeness — GMS2.3+

- ✅ `Dup(N)` byte-based semantics (duplicates N+1 type-sized chunks, not items)
- ✅ DupExtra swap mode (high byte of N, reorders stack without duplicating)
- ✅ Break signals: pushref (-11), chknullish (-10), isstaticok (-6), setstatic/savearef/restorearef (-9/-8/-7)
- ✅ Shared blob child function length computation (gap-based from sorted offsets)
- ✅ Anonymous functions as closures (pushref → GlobalRef)
- ⚠️ Struct literal construction — basic support, edge cases possible
- ⚠️ Sequences API — not yet extracted from data.win
- ⚠️ Animation curves API — not yet extracted

### Transform pass completeness

All standard passes run. GML-specific:
- ✅ `GameMaker.*` system calls resolved to runtime function imports
- ✅ `int`/`uint`/`real`/`string` casts mapped to `gamemaker/math.ts`
- ✅ Global variable reads mapped to `GameMaker.Global.get`
- ⚠️ No GML-specific boolean detection pass yet (see below)

### Output quality

Key remaining issues:
- **Numeric booleans** — GML compiles `if (self.active)` as `push; pushi 1; cmp.eq; bf`. Output is `=== 1` / `=== 0` instead of bare boolean tests. Requires heuristic: identify fields only assigned 0/1/true/false across all functions.
- **Enum detection** — Many GML games use string or numeric constants as enums. Could extract into `const` objects. Reference code uses `Advantages.none`, `MouseButtons.pressed`, etc.
- **Type inference gaps** — GML is dynamically typed; some field/return types remain `: any` without cross-function analysis.

### Runtime coverage

See **[API-COVERAGE-GML.md](../../API-COVERAGE-GML.md)** for full per-function tracking.

High-level gaps:

| Category | Coverage |
|----------|----------|
| Variable functions (struct, instance reflection) | ⚠️ Partial — global done, instance/struct not started |
| Sprites | ⚠️ Partial — basic get_* done, add/replace/delete not started |
| Audio | ⚠️ Type sigs only — full Web Audio integration pending |
| Paths / Timelines | ❌ Not started |
| Cameras & Views | ❌ Not started |
| Physics (Box2D) | ❌ Not started |
| Particles | ❌ Not started |
| Surfaces | ⚠️ Type sigs only |
| GPU / Shaders | ❌ Not started |
| Buffers | ❌ Not started |
| Networking | ❌ Not started |
| Date/Time functions | ❌ Not started |
| Data Structures (ds_list, ds_map, ds_grid) | ⚠️ Type sigs only |
| Async functions | ❌ Not started (mostly irrelevant for offline lifting) |
| Gamepad input | ❌ Not started |
| Time Sources (GMS2.3+) | ❌ Not started |
| Sequences / Animation Curves (GMS2.3+) | ❌ Not started |

### Platform abstraction gaps

- Audio — no platform module; no Web Audio integration
- Persistence/Save — INI uses localStorage directly, not abstracted
- Networking — no platform module
- Dialog/Modal — no platform module

## Known Limitations

- **GMS2.3+ structs** — Anonymous structs and method binding are partially supported
- **Room transitions** — room_goto / room_restart modeled but room initialization order may differ from original
- **Instance deactivation** — `instance_deactivate_*` / `instance_activate_*` (for off-screen culling) not implemented
- **Paths** — The path following system (`path_start`/`path_end` + built-in path vars) has field stubs but no path execution logic
- **Collision system** — Collision events dispatched but spatial queries (`place_meeting`, `collision_rectangle`, etc.) are type-sig stubs only

## References

- [GML Reference (GameMaker Manual)](https://manual.gamemaker.io/lts/en/GameMaker_Language/GML_Reference/GML_Reference.htm)
- [UndertaleModTool](https://github.com/UndertaleMod/UndertaleModTool) — data.win editor and bytecode reference
- [API Coverage Tracker](../../API-COVERAGE-GML.md)
