# Narrat

**Status: Planned** — No implementation started.

## Format

Narrat games are web apps. Scripts are written in **Narrat Script** (`.narrat` files), a YAML-like indented DSL:

```narrat
main:
  talk player idle "Hello, world."
  choice:
    "Option A":
      talk player idle "You chose A."
    "Option B":
      talk player idle "You chose B."
  jump other_scene

other_scene:
  set data.score += 10
  talk player idle "Score: {data.score}"
```

- Labels (`main:`, `other_scene:`) — scene/function entry points
- `talk <character> <animation> "text"` — dialogue
- `choice:` — branching choice block
- `jump <label>` — unconditional jump
- `set <variable> <value>` — variable assignment (supports `+=`, `-=`, etc.)
- `if <condition>:` / `else:` — conditionals
- `run <label>` — subroutine call (returns)
- Interpolation: `{data.varName}` in strings

Variables live in a typed store (`data.*`, `skills.*`, `quests.*`). The engine is Vue 3 + TypeScript under the hood.

## Runtime

Narrat games ship as Vite/Vue web apps. The runtime is open source (MIT). The script interpreter is a simple statement-by-statement executor with a call stack for `run`/`return`.

## Lifting Strategy

Full recompilation (Tier 2).

1. Parse `.narrat` files (indentation-based parser)
2. Labels → IR functions; `jump` → tail call / `Op::Br`; `run` → `Op::Call`
3. `talk` → `SystemCall("Narrat.Say", character, text)` + `Yield`
4. `choice` → `SystemCall("Narrat.Choice", options)` + `Yield` + branch on result
5. `set` → `Op::Store` on typed global slots
6. String interpolation → IR string concat ops

The format is simple and well-documented — this is one of the more tractable frontends.

## What Needs Building

- [ ] `.narrat` indentation parser
- [ ] IR emitter: labels → functions, statements → ops
- [ ] String interpolation → IR string concat
- [ ] `SystemCall` namespace: `Narrat.Say`, `Narrat.Choice`, `Narrat.Notify`
- [ ] Skill check / quest system stubs
- [ ] Replacement runtime (`runtime/narrat/ts/`)

## References

- [Narrat source (MIT)](https://github.com/liana-p/narrat-engine)
- [Narrat documentation](https://docs.narrat.dev)
