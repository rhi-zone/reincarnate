/** SugarCube-specific jQuery prototype extensions.
 *
 * Patches $.fn with methods that SugarCube adds to jQuery:
 * - .wiki(markup) — parse SugarCube markup into the element
 * - .wikiWithOptions(opts, markup) — same with options
 *
 * Call installExtensions() after jQuery is on globalThis.
 */

export function installExtensions(): void {
  const $ = (globalThis as any).jQuery;
  if (!$ || !$.fn) return;

  $.fn.wiki = function (markup: string) {
    return this.each(function () {
      (this as Element).appendChild(document.createTextNode(markup));
    });
  };

  $.fn.wikiWithOptions = function (_opts: any, markup: string) {
    return this.wiki(markup);
  };
}
