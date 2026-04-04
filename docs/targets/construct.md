# Construct 2 / Construct 3

**Status: Planned** — No implementation started.

## Format

Construct games use an **event sheet** programming model — no traditional scripting language. Logic is expressed as condition/action pairs in XML.

### Construct 2 (`.capx`)
A renamed ZIP containing:
- `project.xml` — event sheets, layouts, object types, behaviors
- `files/`, `images/`, `sounds/` — assets

### Construct 3 (`.c3p`)
Also a ZIP:
- `project.c3proj` — JSON project descriptor
- `event sheets/` — `.xml` event sheet files
- `object types/`, `layouts/` — JSON descriptors

Event sheet structure (XML):
```xml
<event-sheet name="Event sheet 1">
  <event-block>
    <conditions>
      <condition id="is-on-screen" object="Sprite" />
    </conditions>
    <actions>
      <action id="set-x" object="Sprite">
        <param>100</param>
      </action>
    </actions>
  </event-block>
  <event-block>
    <sub-events> ... </sub-events>
  </event-block>
</event-sheet>
```

Expressions inside conditions and actions use Construct's expression language (typed, no side effects, object property access via `Object.Variable`).

## Runtime

Event-driven game loop. Each tick: evaluate all event conditions top-to-bottom, execute matching actions. Sub-events nest conditionally. The model is closer to a rule engine than a procedural language.

Behaviors (like Platform, Bullet, Sine) are composable components that run per-tick automatically.

## Lifting Strategy

Full recompilation (Tier 2). The event-sheet model maps to IR as follows:

1. Parse event sheets (XML for C2, XML/JSON for C3)
2. Each event sheet becomes an IR function called once per tick
3. Each event block: conditions → IR branch; actions → IR ops
4. Sub-events → nested IR branches
5. Object variables → `Op::GetField`/`Op::SetField` on typed instances
6. Behaviors → IR functions registered per behavior type
7. Expressions → lifted to IR ops (typed, straightforward)

The condition/action structure is essentially `if (cond) { action; }` chains — this maps directly to IR control flow with no ambiguity.

## What Needs Building

- [ ] `.capx` / `.c3p` parser (ZIP extraction + XML/JSON reader)
- [ ] Event sheet → IR lowering: conditions → branches, actions → ops
- [ ] Expression language → IR ops
- [ ] Object/instance model → IR struct types + `Op::GetField`/`Op::SetField`
- [ ] Behavior registry (Platform, Bullet, etc.) → IR function stubs
- [ ] Replacement runtime (`runtime/construct/ts/`) — game loop, renderer stubs

## References

- [Construct 2 manual](https://www.construct.net/en/make-games/manuals/construct-2)
- [Construct 3 manual](https://www.construct.net/en/make-games/manuals/construct-3)
- [Construct 3 project format](https://www.construct.net/en/make-games/manuals/construct-3/project-primitives/projects)
