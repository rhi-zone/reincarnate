# CLAUDE.md

Behavioral rules for Claude Code in this repository.

## Overview

Winnow is a legacy software lifting framework. It extracts and transforms applications from obsolete runtimes (Flash, Director, VB6, HyperCard, RPG Maker, etc.) into modern web-based equivalents.

### Key Components

- **Explant**: Bytecode/script extraction and decompilation
- **Hypha**: Game/app translation with UI overlay replacement

### The "Lift" Strategy

**Tier 1: Native Patching** - For binaries you can't lift (hex editing, font replacement)
**Tier 2: Runtime Replacement** - For engines you can shim (hook draw calls, render via HTML/CSS overlay)

### Supported Targets

Winnow works on **bytecode and script**, not native code:
- Flash (ABC bytecode)
- Director/Shockwave (Lingo)
- VB6 (P-Code)
- Java Applets (JVM bytecode)
- Silverlight (.NET IL)
- HyperCard/ToolBook (stack formats)
- RPG Maker / Ren'Py / GameMaker

## Core Rule

**Note things down immediately:**
- Bugs/issues → fix or add to TODO.md
- Design decisions → docs/ or code comments
- Future work → TODO.md
- Key insights → this file

**Triggers:** User corrects you, 2+ failed attempts, "aha" moment, framework quirk discovered → document before proceeding.

**Do the work properly.** When asked to analyze X, actually read X - don't synthesize from conversation.

## Design Principles

**Unify, don't multiply.** One interface for multiple engines > separate implementations per engine. Plugin systems > hardcoded switches.

**Lazy extraction.** Don't parse everything upfront. Extract on demand, cache aggressively.

**Preserve fidelity.** The goal is accurate reproduction, not "improvement". Make the old thing work, don't redesign it.

**Overlay > Patch.** When possible, render a modern UI layer over the original rather than patching internal rendering.

**Two-tier approach.** Accept that some targets need binary patching (Tier 1) while others can be fully lifted (Tier 2). Design APIs that work for both.

## Negative Constraints

Do not:
- Announce actions ("I will now...") - just do them
- Leave work uncommitted
- Create special cases - design to avoid them
- Add to the monolith - split by domain into sub-crates
- Cut corners with fallbacks - implement properly for each case
- Mark as done prematurely - note what remains

## Crate Structure

All crates use the `rhizome-winnow-` prefix:
- `rhizome-winnow-core` - Core types and traits
- `rhizome-winnow-cli` - CLI binary (named `winnow`)
- `rhizome-winnow-flash` - Flash/SWF support
- `rhizome-winnow-director` - Director/Shockwave support
- etc.
