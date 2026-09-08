// The parser is a build step: if it breaks, users get an empty or wrong
// "What's new" panel and nothing else fails. So these run against the REAL
// CHANGELOG.md, not a fixture — a reformat of the actual file is exactly
// the regression worth catching.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { isNewer } from "./version";
import {
  parseChangelog,
  parseInline,
  SKIP_MARKER,
  USER_FACING,
} from "./changelog-parse";

const REAL = readFileSync(join(process.cwd(), "CHANGELOG.md"), "utf8");

describe("parseInline", () => {
  it("splits bold and code out of plain text", () => {
    expect(parseInline("A **bold** and `code` bit")).toEqual([
      { text: "A " },
      { text: "bold", bold: true },
      { text: " and " },
      { text: "code", code: true },
      { text: " bit" },
    ]);
  });

  it("flattens links to their text (no href reaches the UI)", () => {
    expect(parseInline("see [the docs](https://example.com/x) now")).toEqual([
      { text: "see the docs now" },
    ]);
  });

  it("reads *italic* without cannibalising **bold**", () => {
    // The changelog leans on **bold** lead-ins heavily; an italic rule that
    // matched the inner halves of a bold pair would wreck every bullet.
    expect(parseInline("**Bold.** then *soft* text")).toEqual([
      { text: "Bold.", bold: true },
      { text: " then " },
      { text: "soft", italic: true },
      { text: " text" },
    ]);
  });

  it("leaves unmatched delimiters literal instead of dropping text", () => {
    expect(parseInline("2 * 3 and a lone ` tick")).toEqual([
      { text: "2 * 3 and a lone ` tick" },
    ]);
  });
});

describe("parseChangelog on the real CHANGELOG.md", () => {
  const notes = parseChangelog(REAL, 12);

  it("parses the file without throwing and returns releases newest-first", () => {
    // Derived from the file rather than hardcoded, so this doesn't need
    // editing every release. It still pins the shape: the newest released
    // heading is the newest note, with a date and a headline.
    const firstHeading = /^## \[(\d+\.\d+\.\d+)\]/m.exec(REAL);
    expect(notes.length).toBe(12);
    expect(notes[0]!.version).toBe(firstHeading![1]);
    expect(notes[0]!.title).toBeTruthy();
    expect(notes[0]!.date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    // Strictly descending. A lexical sort would be wrong here ("1.9.0"
    // sorts above "1.48.0"), so compare semver-wise.
    for (let i = 1; i < notes.length; i += 1) {
      expect(
        isNewer(notes[i - 1]!.version, notes[i]!.version),
        `${notes[i - 1]!.version} should precede ${notes[i]!.version}`,
      ).toBe(true);
    }
  });

  it("never emits [Unreleased]", () => {
    // A phantom empty version would be shown to every user on every update.
    expect(REAL).toContain("## [Unreleased]");
    for (const n of notes) expect(n.version).toMatch(/^\d+\.\d+\.\d+$/);
  });

  it("keeps only user-facing sections", () => {
    // The real file has 61 "### Tests" and 12 "### Pre-push checklist"
    // sections. None of that is for users.
    expect(REAL).toContain("### Tests");
    expect(REAL).toContain("### Pre-push checklist");
    const kinds = new Set(notes.flatMap((n) => n.sections.map((s) => s.kind)));
    for (const k of kinds) expect(USER_FACING).toContain(k);
  });

  it("carries real content for every release it emits", () => {
    for (const n of notes) {
      const bullets = n.sections.reduce((a, s) => a + s.items.length, 0);
      expect(bullets + (n.lead ? 1 : 0), `${n.version} is empty`).toBeGreaterThan(0);
    }
  });

  it("honours the skip marker on internal releases", () => {
    // v1.45.0 is the shared-envelope-follower refactor: "No audible change".
    expect(REAL).toContain(SKIP_MARKER);
    expect(notes.map((n) => n.version)).not.toContain("1.45.0");
  });

  it("respects the limit", () => {
    expect(parseChangelog(REAL, 3)).toHaveLength(3);
  });
});

describe("parseChangelog structure", () => {
  const SAMPLE = [
    "# Changelog",
    "",
    "## [Unreleased]",
    "",
    "## [2.0.0] — 2026-01-02 — Big one",
    "",
    "An intro paragraph",
    "that wraps across lines.",
    "",
    "### Added",
    "",
    "- **Thing.** It does stuff.",
    "  - a nested detail",
    "",
    "### Tests",
    "",
    "- not for users",
    "",
    "## [1.9.0] — 2026-01-01 — Internal",
    "",
    SKIP_MARKER,
    "",
    "### Changed",
    "",
    "- plumbing",
    "",
    "## [1.8.0] — 2025-12-31 — Dev only",
    "",
    "### Tests",
    "",
    "- only tests here",
    "",
  ].join("\n");

  it("captures the lead paragraph and joins wrapped lines", () => {
    const [note] = parseChangelog(SAMPLE);
    expect(note!.lead).toEqual([
      { text: "An intro paragraph that wraps across lines." },
    ]);
  });

  it("nests sub-bullets under their parent", () => {
    const [note] = parseChangelog(SAMPLE);
    const item = note!.sections[0]!.items[0]!;
    expect(item.spans[0]).toEqual({ text: "Thing.", bold: true });
    expect(item.sub).toEqual([{ spans: [{ text: "a nested detail" }] }]);
  });

  it("drops a release whose only content is developer-facing", () => {
    // v1.8.0 has just a Tests section — it must not appear as an empty card.
    expect(parseChangelog(SAMPLE).map((n) => n.version)).toEqual(["2.0.0"]);
  });

  it("ignores a \"## \" line inside a fenced code block", () => {
    // Otherwise a future entry showing sample changelog markup, or a shell
    // comment, would be read as a version heading and throw the build.
    const md = [
      "## [1.0.0] — 2026-01-01 — Real",
      "",
      "### Added",
      "",
      "- a thing",
      "",
      "```sh",
      "## not a heading",
      "```",
      "",
    ].join("\n");
    const notes = parseChangelog(md);
    expect(notes.map((n) => n.version)).toEqual(["1.0.0"]);
  });

  it("only skips on the marker alone on its line", () => {
    // A bullet that merely mentions the marker (documenting it) must not
    // make the release disappear.
    const md = [
      "## [1.0.0] — 2026-01-01 — Real",
      "",
      "### Added",
      "",
      "- Mark internal entries with `" + SKIP_MARKER + "` to hide them.",
      "",
    ].join("\n");
    expect(parseChangelog(md).map((n) => n.version)).toEqual(["1.0.0"]);
  });

  it("throws on a heading it cannot read, rather than skipping it", () => {
    // Fails the BUILD on a CHANGELOG reformat instead of quietly shipping
    // users a panel that's missing releases.
    expect(() => parseChangelog("## [oops] — nope\n")).toThrow(
      /unrecognised version heading/,
    );
  });
});
