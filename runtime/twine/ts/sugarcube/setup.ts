// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** SugarCube `setup` namespace — game-author populated read-only data.
 *
 * In SugarCube, `setup.*` holds game configuration data (item lists, lookup
 * tables, etc.) that is set once during StoryInit and then read-only.
 * This class backs the `SugarCube.Setup.get/set` SystemCalls emitted by the
 * translator for `setup.X` property accesses.
 */
// HANDWRITTEN
export class SCSetup {
  private readonly store: Record<string, unknown> = {};

  get(name: string): unknown {
    return this.store[name];
  }

  set(name: string, value: unknown): void {
    this.store[name] = value;
  }
}
