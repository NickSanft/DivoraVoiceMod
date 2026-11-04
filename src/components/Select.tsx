// Select — 42px field with a leading sigil and a popover dropdown.
// Used for device pickers, preset glyph dropdowns, etc.
//
// v1.60.0: named, honest about a value it doesn't list, and usable from the
// keyboard (D28), for Settings → Hourglass's action pickers, and with them
// every other host (Settings' device, glyph and MIDI pickers, the Mixer's
// recording card, the first-run Wizard's device pickers):
//   - `ariaLabel` makes the trigger's accessible name "{ariaLabel}: {shown
//     value}". The visible text alone names no purpose (a card holds many
//     pickers, and each must name its timer), and an `aria-label` of the
//     purpose alone would hide the value.
//   - A value no option has shows `missing` (else "—"), never the first
//     option: that would name a preset or a clip the control doesn't act on.
//   - Keys, by the active-descendant model: the focus stays on the trigger,
//     which points `aria-activedescendant` at the highlighted option. Enter,
//     Space or Down opens it on the chosen option; Up, Down, Home and End
//     move; Enter or Space picks; Esc closes; Tab closes and moves on. A
//     key's open or move scrolls the highlight into view (`revealOption`):
//     the focus never moves to it, so nothing else would, and a list longer
//     than the window would be picked from blind.
//   - `blurOnPointer` hands the focus back to the page after a mouse or
//     touch click, so Space reaches push-to-modulate again; from the
//     keyboard the focus stays (blurOnPointer.ts).
//
// v1.61.0, for Talk to OBS's pickers:
//   - `onOpen`, called each time the list opens, by a click or a key, so a
//     picker reads OBS's sources again.
//   - An option's `disabled` (a text source that reads a file, or one
//     another timer shows): `aria-disabled="true"`, dimmed, its reason in
//     its `sub`, which its name carries. It stays in the arrow and Home/End
//     order, so the keys reach it and a screen reader says why; Enter, Space
//     and a click refuse it, and the list stays open. A stored value whose
//     option is disabled shows and highlights as any other.
//   - Options that change while the list is open (onOpen's read answering,
//     or OBS's scenes changing) keep the highlight on the option it was on,
//     found by its value, so Enter never picks whatever moved into its
//     place; one that went leaves it at the same place, within the list.
//   - With `ariaLabel`, the trigger's shown sub (which the name leaves out)
//     is its description, with `describedBy`'s lines after it.

import type { JSX } from "solid-js";
import { createEffect, createSignal, createUniqueId, For, on, onCleanup, onMount, Show } from "solid-js";
import { blurOnPointer } from "./blurOnPointer";
import { Sigil, type SigilName } from "./Sigil";

export interface SelectOption {
  value: string;
  label: string;
  sub?: string;
  /** v1.61.0: it can't be picked (a text source that reads a file, or one
   *  another timer shows); `sub` says why. Reachable by the keys, refused
   *  by Enter, Space and a click. */
  disabled?: boolean;
}

export interface SelectProps {
  icon?: SigilName;
  value: string;
  options: SelectOption[];
  onChange: (next: string) => void;
  /** Subtitle shown when the current option has no sub of its own. */
  sub?: string;
  /** v1.60.0: what the picker is for. The trigger's accessible name is then
   *  "{ariaLabel}: {shown value}". Without it the name is the visible text,
   *  as before. */
  ariaLabel?: string;
  /** v1.60.0: what the field shows for a value no option has ("Missing
   *  preset"); "—" without it. */
  missing?: string;
  /** v1.60.0: a pointer click leaves the focus on the page, not on the
   *  trigger. */
  blurOnPointer?: boolean;
  /** v1.61.0: called each time the list opens, by a click or a key. */
  onOpen?: () => void;
  /** v1.61.0: the ids of lines that say more about the value (an OBS
   *  picker's "OBS can't find this source now…"), its description after
   *  the shown sub. */
  describedBy?: string;
}

/** v1.60.0: where an open listbox's highlight goes for a key: Down and Up
 *  one step (stopping at the ends), Home the first option, End the last;
 *  any other key leaves it. Shared with Settings → Hourglass's Add action. */
export function listboxStep(key: string, at: number, count: number): number {
  if (count <= 0) return 0;
  switch (key) {
    case "ArrowDown":
      return Math.min(count - 1, at + 1);
    case "ArrowUp":
      return Math.max(0, at - 1);
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return at;
  }
}

/** The keys an open listbox takes for its highlight. */
export const LISTBOX_MOVES: readonly string[] = ["ArrowDown", "ArrowUp", "Home", "End"];

/** v1.60.0: scrolls an open listbox's highlighted option (by its id) into
 *  view, the least it takes. Only from a key's open or move: a pointer's
 *  hover highlights too, and scrolling then would slide the next option
 *  under a still pointer, which highlights it and scrolls again. Shared
 *  with Settings → Hourglass's Add action. */
export function revealOption(id: string): void {
  document.getElementById(id)?.scrollIntoView?.({ block: "nearest" });
}

export function Select(props: SelectProps): JSX.Element {
  const [open, setOpen] = createSignal(false);
  // The highlighted option while open: where Enter or Space picks.
  const [active, setActive] = createSignal(0);
  let rootRef: HTMLDivElement | undefined;
  let trigger: HTMLButtonElement | undefined;
  const uid = createUniqueId();
  const listId = `select-${uid}-list`;
  const subId = `select-${uid}-sub`;
  const optionId = (i: number): string => `select-${uid}-opt-${i}`;

  onMount(() => {
    const handler = (e: PointerEvent) => {
      if (rootRef && !rootRef.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", handler);
    onCleanup(() => document.removeEventListener("pointerdown", handler));
  });

  // Never the first option for a value none has: that would name a target
  // the control doesn't act on.
  const current = () => props.options.find((o) => o.value === props.value);
  const shown = (): string => current()?.label ?? props.missing ?? "—";
  const sub = (): string | undefined => current()?.sub ?? props.sub;
  const icon = () => props.icon ?? "mic";
  // An `aria-label` names the trigger without its sub, so the sub (and the
  // host's lines) describe it. Without one the sub is already in the name.
  const describedBy = (): string | undefined =>
    [props.ariaLabel !== undefined && sub() ? subId : "", props.describedBy ?? ""].filter((x) => x !== "").join(" ") ||
    undefined;

  /** Opens it with the chosen option highlighted (the first without one). */
  const openList = (): void => {
    const at = props.options.findIndex((o) => o.value === props.value);
    setActive(at >= 0 ? at : 0);
    setOpen(true);
    props.onOpen?.();
  };
  // The options changed while open: the highlight stays on its option, by
  // value (an index would now name another), or, that option gone, at the
  // same place within the list. (Not `defer`: `on` would then hand the
  // first change no `prev`.)
  createEffect(
    on(
      () => props.options,
      (next, prev) => {
        if (!open() || prev === undefined) return;
        const was = prev[active()]?.value;
        const at = was === undefined ? -1 : next.findIndex((o) => o.value === was);
        setActive(at >= 0 ? at : Math.max(0, Math.min(active(), next.length - 1)));
      },
    ),
  );
  /** Picks option `i` and closes; a disabled one is refused, and the list
   *  stays open. True when it picked. */
  const pick = (i: number): boolean => {
    const o = props.options[i];
    if (o?.disabled) return false;
    setOpen(false);
    if (o) props.onChange(o.value);
    return true;
  };

  const onKeyDown = (e: KeyboardEvent): void => {
    if (!open()) {
      if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown") {
        // Handled here, so the button's own click doesn't open it again.
        e.preventDefault();
        if (props.options.length > 0) {
          openList();
          revealOption(optionId(active()));
        }
      }
      return;
    }
    if (LISTBOX_MOVES.includes(e.key)) {
      e.preventDefault();
      setActive(listboxStep(e.key, active(), props.options.length));
      revealOption(optionId(active()));
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      pick(active());
    } else if (e.key === "Escape") {
      e.preventDefault();
      setOpen(false);
    } else if (e.key === "Tab") {
      // Closes, and the focus moves on as Tab says.
      setOpen(false);
    }
  };

  return (
    <div ref={rootRef} style={{ position: "relative" }}>
      <button
        ref={trigger}
        type="button"
        class={`select ${open() ? "is-open" : ""}`}
        onClick={(e) => {
          if (props.blurOnPointer) blurOnPointer(e);
          if (open()) setOpen(false);
          else openList();
        }}
        onKeyDown={onKeyDown}
        // A Space released on the trigger would click it once more.
        onKeyUp={(e) => {
          if (e.key === " ") e.preventDefault();
        }}
        aria-haspopup="listbox"
        aria-expanded={open()}
        aria-controls={open() ? listId : undefined}
        aria-activedescendant={open() && props.options.length > 0 ? optionId(active()) : undefined}
        aria-label={props.ariaLabel !== undefined ? `${props.ariaLabel}: ${shown()}` : undefined}
        aria-describedby={describedBy()}
      >
        <span class="select-icon">
          <Sigil name={icon()} size={18} />
        </span>
        <span class="select-text">
          <div class="select-main">{shown()}</div>
          <Show when={sub()}>
            <div id={subId} class="select-sub">
              {sub()}
            </div>
          </Show>
        </span>
        <Sigil
          name="chevronD"
          size={16}
          style={{
            color: "var(--text-lo)",
            transform: open() ? "rotate(180deg)" : "none",
            transition: "transform 0.16s",
          }}
        />
      </button>
      <Show when={open()}>
        <div
          id={listId}
          class="dropdown"
          role="listbox"
          aria-label={props.ariaLabel}
          style={{
            position: "absolute",
            top: "calc(100% + 6px)",
            left: 0,
            right: 0,
            "z-index": 50,
          }}
        >
          <For each={props.options}>
            {(o, i) => (
              <div
                id={optionId(i())}
                class={`dropdown-opt ${o.value === props.value ? "is-selected" : ""}`}
                role="option"
                aria-selected={o.value === props.value}
                aria-disabled={o.disabled ? "true" : undefined}
                style={{
                  ...(i() === active() ? { outline: "1px solid var(--line-glow)", "outline-offset": "-1px" } : {}),
                  ...(o.disabled ? { opacity: 0.55, cursor: "default" } : {}),
                }}
                onPointerEnter={() => setActive(i())}
                onClick={(e) => {
                  if (!pick(i())) return;
                  // A pointer's pick leaves the focus on the page.
                  if (props.blurOnPointer && e.detail > 0) trigger?.blur();
                }}
              >
                <Sigil
                  name={icon()}
                  size={16}
                  style={{
                    color: o.value === props.value ? "var(--indigo)" : "var(--text-lo)",
                  }}
                />
                <div style={{ flex: 1 }}>
                  <div style={{ "font-size": "var(--t-sm)", "font-weight": 600 }}>
                    {o.label}
                  </div>
                  <Show when={o.sub}>
                    <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
                      {o.sub}
                    </div>
                  </Show>
                </div>
                <Show when={o.value === props.value}>
                  <Sigil name="check" size={15} style={{ color: "var(--indigo)" }} />
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
