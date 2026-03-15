/** SugarCube `setup` namespace — game-author populated read-only data.
 *
 * In SugarCube, `setup.*` holds game configuration data (item lists, lookup
 * tables, etc.) that is set once during StoryInit and then read-only.
 * This class backs the `SugarCube.Setup.get/set` SystemCalls emitted by the
 * translator for `setup.X` property accesses.
 */
export class SCSetup {
  private readonly store: Record<string, unknown> = {};

  get(name: string): unknown {
    return this.store[name];
  }

  set(name: string, value: unknown): void {
    this.store[name] = value;
  }
}
