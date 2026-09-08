// v1.49.0: "What's new" — a post-update banner and the panel it opens.
//
// Content is CHANGELOG.md parsed at build time (src/data/changelog.ts), so
// there is no fetch, no loading state, and no way for the notes to describe
// a release the user isn't running.
//
// Deliberately NOT a launch modal. The first-run wizard is the one thing
// entitled to block, and it blocks once; this is a voice modulator whose
// launch path is usually someone about to get on mic. A dialog the user
// opens is fine — one that opens itself is not.

import { For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Button } from "./Button";
import { IconButton } from "./IconButton";
import { Sigil } from "./Sigil";
import { useApp } from "../stores/app";
import { CHANGELOG_URL, type ReleaseNote } from "../data/changelog";
import type { Bullet, Span } from "../data/changelog-parse";
import { openExternal } from "../lib/openExternal";

const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

/** "2026-09-04" -> "4 September 2026". Hand-rolled rather than
 *  toLocaleDateString so the output doesn't shift with the host locale. */
function formatDate(iso: string | null): string {
  if (!iso) return "";
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  if (!m) return iso;
  const month = MONTHS[Number(m[2]) - 1];
  if (!month) return iso;
  return `${Number(m[3])} ${month} ${m[1]}`;
}

/** Render parsed inline spans as real elements — no markdown library and
 *  no innerHTML, so changelog text can never inject markup. */
function Spans(props: { spans: Span[] }): JSX.Element {
  return (
    <For each={props.spans}>
      {(s) => (
        <Show
          when={s.bold}
          fallback={
            <Show
              when={s.code}
              fallback={
                <Show when={s.italic} fallback={<>{s.text}</>}>
                  <em>{s.text}</em>
                </Show>
              }
            >
              <code
                class="mono"
                style={{
                  "font-size": "0.92em",
                  padding: "1px 5px",
                  "border-radius": "4px",
                  background: "var(--surface-2)",
                }}
              >
                {s.text}
              </code>
            </Show>
          }
        >
          <strong style={{ color: "var(--text-hi)" }}>{s.text}</strong>
        </Show>
      )}
    </For>
  );
}

function Bullets(props: { items: Bullet[] }): JSX.Element {
  return (
    <ul
      style={{
        margin: 0,
        // The global reset clears list markers; restore them here so a
        // section of several short fixes reads as discrete items.
        "list-style": "disc outside",
        padding: "0 0 0 var(--s5)",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
      }}
    >
      <For each={props.items}>
        {(b) => (
          <li style={{ "line-height": 1.6 }}>
            <Spans spans={b.spans} />
            <Show when={b.sub}>
              {(sub) => (
                <ul
                  style={{
                    margin: "var(--s2) 0 0",
                    "list-style": "circle outside",
                    padding: "0 0 0 var(--s4)",
                    color: "var(--text-lo)",
                  }}
                >
                  <For each={sub()}>
                    {(s) => (
                      <li>
                        <Spans spans={s.spans} />
                      </li>
                    )}
                  </For>
                </ul>
              )}
            </Show>
          </li>
        )}
      </For>
    </ul>
  );
}

function ReleaseCard(props: { note: ReleaseNote }): JSX.Element {
  return (
    <article
      style={{
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
        "padding-bottom": "var(--s5)",
        "border-bottom": "1px solid var(--line)",
      }}
    >
      <header
        style={{ display: "flex", "align-items": "baseline", gap: "var(--s3)" }}
      >
        <h3
          class="display"
          style={{
            margin: 0,
            "font-size": "var(--t-h3)",
            color: "var(--text-hi)",
          }}
        >
          {props.note.title ?? `Version ${props.note.version}`}
        </h3>
        <span
          class="mono"
          style={{ "font-size": "var(--t-xs)", color: "var(--indigo)" }}
        >
          v{props.note.version}
        </span>
        <span style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
          {formatDate(props.note.date)}
        </span>
      </header>
      <Show when={props.note.lead}>
        {(lead) => (
          <p style={{ margin: 0, color: "var(--text-mid)", "line-height": 1.6 }}>
            <Spans spans={lead()} />
          </p>
        )}
      </Show>
      <For each={props.note.sections}>
        {(section) => (
          <div
            style={{
              display: "flex",
              "flex-direction": "column",
              gap: "var(--s2)",
            }}
          >
            <div class="eyebrow">{section.kind}</div>
            <div style={{ color: "var(--text-mid)" }}>
              <Bullets items={section.items} />
            </div>
          </div>
        )}
      </For>
    </article>
  );
}

export function WhatsNewModal(): JSX.Element {
  const app = useApp();

  onMount(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape" && app.whatsNewOpen()) app.closeWhatsNew();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    <Show when={app.whatsNewOpen() && app.whatsNewContent()}>
      {(content) => (
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby="whats-new-title"
          style={{
            position: "fixed",
            inset: 0,
            background: "var(--scrim)",
            "backdrop-filter": "blur(4px)",
            display: "grid",
            "place-items": "center",
            "z-index": 200,
          }}
          onClick={(e) => {
            if (e.currentTarget === e.target) app.closeWhatsNew();
          }}
        >
          <div
            class="panel"
            style={{
              width: "680px",
              "max-width": "calc(100vw - 48px)",
              "max-height": "calc(100vh - 96px)",
              display: "flex",
              "flex-direction": "column",
              background: "var(--surface-1)",
              "box-shadow": "var(--shadow-3)",
            }}
          >
            <div
              style={{
                display: "flex",
                "align-items": "center",
                gap: "var(--s3)",
                padding: "var(--s4) var(--s5)",
                "border-bottom": "1px solid var(--line)",
              }}
            >
              <Sigil name="bolt" size={18} style={{ color: "var(--indigo)" }} />
              <h2
                id="whats-new-title"
                class="display"
                style={{ "font-size": "var(--t-h3)", flex: 1, margin: 0 }}
              >
                What&rsquo;s new
              </h2>
              <IconButton icon="x" onClick={app.closeWhatsNew} tip="Close" />
            </div>

            <div
              style={{
                flex: 1,
                "min-height": 0,
                overflow: "auto",
                padding: "var(--s5)",
                display: "flex",
                "flex-direction": "column",
                gap: "var(--s5)",
              }}
            >
              <For each={content().notes}>
                {(note) => <ReleaseCard note={note} />}
              </For>
              <Show when={content().moreCount > 0 || content().moreUnknown}>
                <p
                  style={{
                    margin: 0,
                    "font-size": "var(--t-sm)",
                    color: "var(--text-lo)",
                  }}
                >
                  {content().moreUnknown
                    ? "…and earlier releases."
                    : `…and ${content().moreCount} earlier release${
                        content().moreCount === 1 ? "" : "s"
                      }.`}
                </p>
              </Show>
            </div>

            <div
              style={{
                display: "flex",
                "align-items": "center",
                gap: "var(--s3)",
                padding: "var(--s4) var(--s5)",
                "border-top": "1px solid var(--line)",
              }}
            >
              <span
                style={{
                  flex: 1,
                  "font-size": "var(--t-xs)",
                  color: "var(--text-lo)",
                }}
              >
                Notes ship with the app — nothing was downloaded.
              </span>
              <Button
                variant="ghost"
                size="sm"
                iconR="external"
                onClick={() => void openExternal(CHANGELOG_URL)}
              >
                Full changelog
              </Button>
              <Button variant="primary" size="sm" onClick={app.closeWhatsNew}>
                Done
              </Button>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
}

/** The post-update notice. Non-blocking and self-effacing: it floats over
 *  the content area rather than reflowing it, and one click either opens
 *  the panel or makes it go away for good.
 *
 *  Sits BELOW the first-run wizard (z-index 100) rather than checking
 *  whether the wizard is open: the two can only coincide when someone
 *  replays setup, and stacking order settles that without coupling this
 *  component to whether the wizard happens to be mounted. */
export function WhatsNewBanner(): JSX.Element {
  const app = useApp();
  return (
    <Show when={app.whatsNewBanner()}>
      {(note) => (
        <div
          class="card"
          role="status"
          style={{
            position: "absolute",
            bottom: "var(--s5)",
            left: "50%",
            transform: "translateX(-50%)",
            "z-index": 90,
            display: "flex",
            "align-items": "center",
            gap: "var(--s3)",
            padding: "var(--s3) var(--s4)",
            "max-width": "calc(100% - 48px)",
            background: "var(--surface-1)",
            "border-color": "var(--line-glow)",
            "box-shadow": "var(--shadow-3)",
          }}
        >
          <span style={{ color: "var(--indigo)", "flex-shrink": 0 }}>
            <Sigil name="bolt" size={18} />
          </span>
          <span
            style={{
              "font-size": "var(--t-sm)",
              color: "var(--text-mid)",
              "min-width": 0,
              overflow: "hidden",
              "text-overflow": "ellipsis",
              "white-space": "nowrap",
            }}
          >
            <strong style={{ color: "var(--text-hi)" }}>
              Updated to v{note().version}
            </strong>
            {note().title ? ` — ${note().title}` : ""}
          </span>
          <Button variant="secondary" size="sm" onClick={app.openWhatsNew}>
            See what&rsquo;s new
          </Button>
          <IconButton
            icon="x"
            onClick={app.dismissWhatsNewBanner}
            tip="Dismiss"
          />
        </div>
      )}
    </Show>
  );
}
