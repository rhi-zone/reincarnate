/** SugarCube navigation (passage transitions). */

/** Navigate to a passage by name or expression. */
export function goto(target: string): void {
  console.log("[goto]", target);
}

/** Go back to the previous passage. */
export function back(): void {
  console.log("[back]");
}

/** Return to a previous passage (like back but pops multiple turns). */
// Using a wrapper to avoid JS reserved word in export.
export { returnNav as return };
function returnNav(): void {
  console.log("[return]");
}

/** Include (embed) another passage inline. */
export function include(passage: string): void {
  console.log("[include]", passage);
}
