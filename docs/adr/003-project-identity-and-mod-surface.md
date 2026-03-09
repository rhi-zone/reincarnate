# ADR 003: Project Identity and Mod Surface

**Date:** 2026-03-10
**Status:** Accepted

## Context

An investigation into the modded Bounty reference decompilation (`~/git/bounty/`) raised the question of whether Reincarnate should include an IR-based modding framework. The mod delta (2262 insertions over 115 files: new UI subsystems, new 495-line encounter script, mutations to existing classes) is clearly logic-heavy — not addressable by a data-only manifest overlay.

This prompted a more fundamental question: what is Reincarnate, and what is the right mod surface?

## Three Candidate Framings

**1. Preservation tool** — make old games run in a browser. Success = playable. TypeScript is incidental.
Implication: interpreter bundling (renpyweb, inkjs, libqsp-WASM) is equally valid. CLAUDE.md rejects this, but the rejection has no justification under this framing.

**2. Decompiler / lifter** — produce editable, maintainable source code from opaque binaries. The TypeScript IS the artifact. Think Ghidra, but the output compiles and runs.
Implication: mods happen in the emitted TypeScript. Re-lifting is a one-time operation per game version. The IR is an internal pipeline detail.

**3. Continuous lift platform** — the original binary is the source of truth. Mods are overlays on IR. Re-lift when upstream updates; mods survive without manual forward-porting.
Implication: requires an IR mutation API. Only makes sense when the upstream game keeps updating.

## Decision

Reincarnate is framing **2**: a decompiler that produces working, type-safe, maintainable code.

This is the framing that makes all the existing laws coherent:
- The prohibition on bundling interpreters: an emulator isn't decompilation.
- Behavioral equivalence: the emitted code must faithfully represent the source.
- Honest representation: IR types reflect source semantics, not VM storage.
- Instantiability: emitted code is a normal program, not a scripted engine.

## Consequences for Modding

The correct modding workflow under this framing is:

1. `reincarnate emit` → get TypeScript
2. Edit the TypeScript

No IR mutation API is needed. The emitted TypeScript is the mod surface — it is exactly as editable as any other TypeScript codebase. If the upstream game updates, mods can be forward-ported via `git cherry-pick` or manual merge, the same way any fork is maintained.

Framing 3 ("semantic modding" — IR overlays that survive re-lifts) is an interesting concept worth exploring eventually as a tangent. But it is explicitly not a priority: lifting more dead engines is far more valuable work. Don't let modding infrastructure concerns influence core pipeline architecture decisions.

The IR-based modding investigation item in TODO.md is closed with this decision.
