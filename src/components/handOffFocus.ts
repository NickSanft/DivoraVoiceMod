// handOffFocus — v1.61.0: a control that goes because of what it did (Try
// again once it's connecting, Forget password once forgotten, a timer's OBS
// row cleared while Settings → OBS is off) would leave the focus on the
// page: a keyboard user loses their place, and a screen reader says
// nothing. Called in the branch that holds the control, this moves the
// focus on as the branch goes, when it is inside `from`: to `to()` when
// that gives one (made focusable for the moment when it isn't, so a status
// line can take it), else to the next control after `from` in Tab order.
// Solid runs a branch's cleanups while its nodes are still in the page, so
// the focus is still there to move. A focus elsewhere is left alone, so a
// pointer click (blurOnPointer) moves nothing.

import { onCleanup } from "solid-js";

const CONTROLS = "button, input, select, textarea, a[href], [tabindex]";

/** The first control after `el` (outside it) that Tab would reach, or null. */
export function nextControl(el: Element): HTMLElement | null {
  for (const c of Array.from(document.querySelectorAll<HTMLElement>(CONTROLS))) {
    if (el.contains(c) || !(el.compareDocumentPosition(c) & Node.DOCUMENT_POSITION_FOLLOWING)) continue;
    if (c.tabIndex < 0 || (c as HTMLButtonElement).disabled) continue;
    return c;
  }
  return null;
}

/** As the calling branch goes, moves the focus from inside `from()` to
 *  `to()`, or else to the next control after it. */
export function handOffFocus(from: () => Element | undefined, to?: () => HTMLElement | undefined): void {
  onCleanup(() => {
    const el = from();
    const at = document.activeElement;
    if (!el || !at || !el.contains(at)) return;
    const next = to?.() ?? nextControl(el);
    if (!next || !next.isConnected || el.contains(next)) return;
    if (next.tabIndex < 0 && !next.hasAttribute("tabindex")) {
      // Focusable for this visit only: it isn't a Tab stop.
      next.setAttribute("tabindex", "-1");
      next.addEventListener("blur", () => next.removeAttribute("tabindex"), { once: true });
    }
    next.focus();
  });
}
