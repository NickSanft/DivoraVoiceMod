// v1.49.0: turns CHANGELOG.md into typed release notes for the in-app
// "What's new" panel.
//
// Runs at BUILD time (see vite/changelog-plugin.ts), never in the app, so
// the shipped bundle carries a small structured array instead of a 215 KB
// markdown file plus a parser. That also means no markdown renderer and no
// `innerHTML`: the UI walks these spans and emits real elements.
//
// Deliberately strict. A heading it cannot read throws rather than being
// skipped, so a future CHANGELOG reformat fails the build instead of
// silently emptying the panel for users.

/** One run of inline text. `bold` and `code` are the only inline markdown
 *  the changelog actually uses; links are flattened to their text. */
export interface Span {
  text: string;
  bold?: true;
  italic?: true;
  code?: true;
}

export interface Bullet {
  spans: Span[];
  /** Nested `  - ` bullets, one level deep (the only depth in use). */
  sub?: Bullet[];
}

/** The user-facing subsection kinds. Everything else in the changelog
 *  (Tests, Pre-push checklist, Architecture notes, …) is developer-facing
 *  and never reaches the app. */
export const USER_FACING = ["Added", "Changed", "Fixed", "Removed"] as const;
export type SectionKind = (typeof USER_FACING)[number];

export interface Section {
  kind: SectionKind;
  items: Bullet[];
}

export interface ReleaseNote {
  /** Bare semver, no leading "v" — e.g. "1.48.0". */
  version: string;
  /** ISO date from the heading; null only for headings that omit it. */
  date: string | null;
  /** The headline after the date — e.g. "Bitcrusher". */
  title: string | null;
  /** Intro prose between the version heading and the first `###`. */
  lead: Span[] | null;
  sections: Section[];
}

/** Marker authors put under a version heading to keep it out of the app.
 *  An HTML comment, so it stays invisible on GitHub. */
export const SKIP_MARKER = "<!-- whatsnew:skip -->";

// Heading shapes in use, oldest to newest:
//   ## [Unreleased]
//   ## [0.0.0] — 2026-05-28
//   ## [1.48.0] — 2026-09-04 — Bitcrusher
const HEADING =
  /^##\s+\[(\d+\.\d+\.\d+)\](?:\s*[—–-]\s*(\d{4}-\d{2}-\d{2}))?(?:\s*[—–-]\s*(.+))?\s*$/;
const UNRELEASED = /^##\s+\[Unreleased\]\s*$/i;

const TOP_BULLET = /^-\s+(.*)$/;
const SUB_BULLET = /^\s{2,}[-*]\s+(.*)$/;
const SECTION_HEADING = /^###\s+(.*)$/;

/** Split inline markdown into spans. Handles `**bold**`, `*italic*` and
 *  `` `code` ``, and flattens `[text](url)` to `text`. Unmatched delimiters
 *  stay literal — a stray asterisk is text, not a dropped character. */
export function parseInline(input: string): Span[] {
  const flat = input.replace(/\[([^\]]+)\]\((?:[^)]*)\)/g, "$1");
  const spans: Span[] = [];
  // Bold first: `**x**` must not be read as an empty italic pair.
  const re = /\*\*([^*]+)\*\*|`([^`]+)`|\*([^*\s][^*]*)\*/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(flat)) !== null) {
    if (m.index > last) spans.push({ text: flat.slice(last, m.index) });
    if (m[1] !== undefined) spans.push({ text: m[1], bold: true });
    else if (m[2] !== undefined) spans.push({ text: m[2], code: true });
    else spans.push({ text: m[3] as string, italic: true });
    last = m.index + m[0].length;
  }
  if (last < flat.length) spans.push({ text: flat.slice(last) });
  return spans.filter((s) => s.text.length > 0);
}

function isUserFacing(heading: string): SectionKind | null {
  // Variants in the wild: "Added", "Changed — voice tuning",
  // "Added — In-app update check". Match the word before any dash.
  const head = (heading.split(/[—–]/)[0] ?? "").trim().toLowerCase();
  return USER_FACING.find((k) => k.toLowerCase() === head) ?? null;
}

interface RawBlock {
  version: string;
  date: string | null;
  title: string | null;
  lines: string[];
}

function splitBlocks(markdown: string): RawBlock[] {
  const lines = markdown.split(/\r?\n/);
  const blocks: RawBlock[] = [];
  let current: RawBlock | null = null;
  let inFence = false;
  for (const line of lines) {
    // A fenced block can legitimately contain a "## " line (a shell
    // comment, a sample changelog). Without this it would be read as a
    // version heading and throw the build.
    if (line.trimStart().startsWith("```")) inFence = !inFence;
    if (inFence || !line.startsWith("## ")) {
      current?.lines.push(line);
      continue;
    }
    if (UNRELEASED.test(line)) {
      // Present in every working tree; must never reach the app as a
      // phantom empty release.
      current = null;
      continue;
    }
    const m = HEADING.exec(line);
    if (!m) {
      throw new Error(
        `[changelog] unrecognised version heading: ${JSON.stringify(line)}\n` +
          `Expected "## [1.2.3] — YYYY-MM-DD — Title" or "## [Unreleased]".`,
      );
    }
    current = {
      version: m[1] as string,
      date: m[2] ?? null,
      title: m[3]?.trim() || null,
      lines: [],
    };
    blocks.push(current);
  }
  return blocks;
}

function parseBlock(block: RawBlock): ReleaseNote | null {
  // Must be the marker ALONE on its line. `includes` would let a bullet
  // that merely mentions the marker (documenting it, say) skip its own
  // release.
  if (block.lines.some((l) => l.trim() === SKIP_MARKER)) return null;

  const lead: string[] = [];
  const sections: Section[] = [];
  let section: Section | null = null;
  let seenHeading = false;
  let bullets: Bullet[] | null = null;

  for (const raw of block.lines) {
    const line = raw.trimEnd();
    const sec = SECTION_HEADING.exec(line);
    if (sec) {
      seenHeading = true;
      const kind = isUserFacing(sec[1] as string);
      if (kind) {
        section = { kind, items: [] };
        sections.push(section);
        bullets = section.items;
      } else {
        section = null;
        bullets = null;
      }
      continue;
    }
    if (line.startsWith("<!--")) continue;

    if (!seenHeading) {
      // Intro prose under the version heading — often the best one-line
      // summary a release has ("The Minecraft-mob series finale").
      if (line.trim().length > 0 && !TOP_BULLET.test(line.trim())) {
        lead.push(line.trim());
      }
      continue;
    }
    if (!bullets) continue;

    const sub = SUB_BULLET.exec(line);
    if (sub) {
      const parent = bullets[bullets.length - 1];
      if (parent) {
        (parent.sub ??= []).push({ spans: parseInline(sub[1] as string) });
      }
      continue;
    }
    const top = TOP_BULLET.exec(line);
    if (top) {
      bullets.push({ spans: parseInline(top[1] as string) });
      continue;
    }
    // Wrapped continuation of the bullet above.
    if (line.trim().length > 0) {
      const parent = bullets[bullets.length - 1];
      const target = parent?.sub?.[parent.sub.length - 1] ?? parent;
      if (target) target.spans.push({ text: ` ${line.trim()}` });
    }
  }

  // A version with nothing user-facing left (all Tests / Architecture) is
  // not worth a heading in the panel.
  const hasBody = sections.some((s) => s.items.length > 0) || lead.length > 0;
  if (!hasBody) return null;

  return {
    version: block.version,
    date: block.date,
    title: block.title,
    lead: lead.length > 0 ? parseInline(lead.join(" ")) : null,
    sections: sections.filter((s) => s.items.length > 0),
  };
}

/** Parse CHANGELOG.md into release notes, newest first.
 *  @param limit keep only the newest N releases (the bundle stays small). */
export function parseChangelog(markdown: string, limit = 12): ReleaseNote[] {
  const notes: ReleaseNote[] = [];
  for (const block of splitBlocks(markdown)) {
    const note = parseBlock(block);
    if (note) notes.push(note);
    if (notes.length >= limit) break;
  }
  return notes;
}
