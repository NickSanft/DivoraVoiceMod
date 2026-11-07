import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { LIGHT_RED } from "./recLight";
import {
  ALIGNS,
  changedFields,
  COLOURS,
  DRAW_MAX_CHARS,
  DRAW_MAX_LINES,
  drawText,
  firstLine,
  fitTextFont,
  fontSpec,
  fontStack,
  FONTS,
  LINE_H,
  MAX_LINES,
  MAX_TEXT,
  normalizeCharmText,
  outlineColor,
  readCharmText,
  readTextFields,
  REF_PX,
  textBox,
  textLength,
  textShadow,
  type TextFont,
  type TextInput,
} from "./text";
import fixture from "./wire.fixture.json";

// wire.fixture.json → `textCharm` holds what a stored text charm may name
// and its creation limits (v1.62.0). STABLE-SURFACE freezes them.

// Characters the table needs, by code, so the source holds no invisible
// ones.
const ch = (code: number): string => String.fromCharCode(code);
const LS = ch(0x2028); // LINE SEPARATOR
const PS = ch(0x2029); // PARAGRAPH SEPARATOR
const NEL = ch(0x85); // NEXT LINE, a C1 control
const BEL = ch(0x07);
const VT = ch(0x0b);
const NUL = ch(0x00);
const US = ch(0x1f);
const DEL = ch(0x7f);
const DCS = ch(0x90);
const APC = ch(0x9f);
const GAME = "🎮"; // one emoji, two UTF-16 code units

describe("the text charm's frozen names and limits (v1.62.0)", () => {
  it("fixture textCharm is text.ts's", () => {
    expect(Object.keys(FONTS)).toEqual(fixture.textCharm.fonts);
    expect(COLOURS).toEqual(fixture.textCharm.colors);
    expect(ALIGNS).toEqual(fixture.textCharm.aligns);
    expect(MAX_TEXT).toBe(fixture.textCharm.maxChars);
    expect(MAX_LINES).toBe(fixture.textCharm.maxLines);
    expect(REF_PX).toBe(fixture.textCharm.refPx);
    expect(LINE_H).toBe(fixture.textCharm.lineHeight);
  });

  it("families are the CSS tokens", () => {
    // The face draws with the same stack the main window's text uses, and
    // the canvas measures with it.
    const css = readFileSync(join(process.cwd(), "src", "styles.css"), "utf8");
    const token = (name: string): string | undefined => css.match(new RegExp(`--${name}:\\s*([^;]+);`))?.[1]?.trim();
    const tokens: Record<TextFont, string> = { display: "font-display", sans: "font-ui", mono: "font-mono" };
    for (const [font, name] of Object.entries(tokens) as Array<[TextFont, string]>) {
      expect(FONTS[font].stack, font).toBe(token(name));
      expect(FONTS[font].stack.startsWith(`"${FONTS[font].family}",`), font).toBe(true);
    }
  });

  it("fontSpec strings: the weight, 100 px and the face's whole stack", () => {
    expect(fontSpec("sans", true)).toBe('700 100px "Space Grotesk Variable", system-ui, sans-serif');
    expect(fontSpec("mono", false)).toBe('400 100px "Space Mono", ui-monospace, monospace');
    for (const font of Object.keys(FONTS) as TextFont[]) {
      expect(fontSpec(font, true)).toBe(`700 100px ${fontStack(font)}`);
      expect(fontSpec(font, false)).toBe(`400 100px ${fontStack(font)}`);
    }
    // A font this build doesn't know (a later build's, or a hand edit) is
    // measured, and drawn, as Sans; a stored string never reaches the spec.
    for (const font of ["serif", "x; color: red", "constructor", 3, undefined]) {
      expect(fontStack(font)).toBe(FONTS.sans.stack);
    }
  });

  it("text.ts imports nothing", () => {
    // scene.ts and the overlay page import it.
    const source = readFileSync(join(process.cwd(), "src", "overlay", "text.ts"), "utf8");
    expect(source).not.toMatch(/^\s*import\b/m);
  });
});

describe("reading a text charm's text (v1.62.0, §7.1)", () => {
  it.each([
    // [name, raw, normalizeCharmText, readCharmText]
    ["plain", "hi", "hi", "hi"],
    ["trimmed", "  hi \n", "hi", "hi"],
    ["CR LF", "a\r\nb", "a\nb", "a\nb"],
    ["CR", "a\rb", "a\nb", "a\nb"],
    ["U+2028 is a newline", `a${LS}b`, "a\nb", "a\nb"],
    ["U+2029 is a newline", `a${PS}b`, "a\nb", "a\nb"],
    ["U+0085 is a newline", `a${NEL}b`, "a\nb", "a\nb"],
    ["a tab is a space", "a\tb", "a b", "a b"],
    ["two tabs are two spaces", "a\t\tb", "a  b", "a  b"],
    ["C0 dropped", `a${BEL}b${VT}c${NUL}d${US}e`, "abcde", "abcde"],
    ["DEL dropped", `a${DEL}b`, "ab", "ab"],
    ["C1 dropped", `a${DCS}b${APC}c`, "abc", "abc"],
    ["U+0085 alone: a newline, trimmed to nothing", NEL, null, null],
    ["only controls and spaces", ` \t${BEL}\n${DEL} `, null, null],
    ["empty", "", null, null],
    ["text, never markup", "<b>hi</b>", "<b>hi</b>", "<b>hi</b>"],
    ["an emoji counts 2", `${GAME} live`, `${GAME} live`, `${GAME} live`],
    ["200 units", "x".repeat(200), "x".repeat(200), "x".repeat(200)],
    ["201 units: live and drawn, but not added or set", "x".repeat(201), "x".repeat(201), null],
    ["100 emoji are 200 units", GAME.repeat(100), GAME.repeat(100), GAME.repeat(100)],
    ["100 emoji and one more unit are 201", `${GAME.repeat(100)}x`, `${GAME.repeat(100)}x`, null],
    ["200 units after the trim", `  ${"x".repeat(200)}  `, "x".repeat(200), "x".repeat(200)],
    ["5 lines", "1\n2\n3\n4\n5", "1\n2\n3\n4\n5", "1\n2\n3\n4\n5"],
    ["6 lines", "1\n2\n3\n4\n5\n6", "1\n2\n3\n4\n5\n6", null],
    ["6 lines by U+2029", ["1", "2", "3", "4", "5", "6"].join(PS), "1\n2\n3\n4\n5\n6", null],
    ["6 lines by CR", "1\r2\r3\r4\r5\r6", "1\n2\n3\n4\n5\n6", null],
    ["trimmed first", "\n\n\nx", "x", "x"],
    ["lines counted after the trim", "\n\n\n\n\n\nx\n\n", "x", "x"],
    ["6 lines after the trim", "a\n\n\n\n\nb", "a\n\n\n\n\nb", null],
    ["a number", 42, null, null],
    ["null", null, null, null],
    ["undefined", undefined, null, null],
  ] as Array<[string, unknown, string | null, string | null]>)("normalize and read: %s", (_name, raw, normal, read) => {
    expect(normalizeCharmText(raw)).toBe(normal);
    expect(readCharmText(raw)).toBe(read);
  });

  it("textLength counts as the limit counts", () => {
    expect(textLength(`${GAME} live`)).toBe(7);
    expect(textLength("a\r\nb")).toBe(3);
    expect(textLength("  hi  ")).toBe(2);
    expect(textLength(GAME.repeat(100))).toBe(MAX_TEXT);
    for (const raw of ["", "  ", 42, null]) expect(textLength(raw)).toBe(0);
  });

  it("drawText ceiling: 50 lines, then 4096 units, never half a pair", () => {
    expect(DRAW_MAX_CHARS).toBe(4096);
    expect(DRAW_MAX_LINES).toBe(50);
    expect(drawText("x".repeat(5000))).toBe("x".repeat(4096));
    // A pair across the cut is left out whole.
    const odd = `a${GAME.repeat(2500)}`;
    const cut = drawText(odd);
    expect(cut).toHaveLength(4095);
    expect(cut).toBe(odd.slice(0, 4095));
    const last = cut.charCodeAt(cut.length - 1);
    expect(last >= 0xd800 && last <= 0xdbff).toBe(false);
    // A pair ending at the cut is kept.
    expect(drawText(GAME.repeat(2500))).toBe(GAME.repeat(2048));
    // Lines first: 60 lines draw 50.
    const sixty = Array.from({ length: 60 }, (_, i) => `line ${i + 1}`).join("\n");
    expect(drawText(sixty).split("\n")).toHaveLength(50);
    expect(drawText(sixty).split("\n").at(-1)).toBe("line 50");
    // Then units: 60 lines of 100 cut to 50 lines (5049 units), then 4096.
    const wide = Array.from({ length: 60 }, () => "y".repeat(100)).join("\n");
    expect(drawText(wide)).toBe(wide.slice(0, 4096));
  });

  it("drawText normalizes, and has no creation limit", () => {
    expect(drawText("  hi\r\nthere\t! ")).toBe("hi\nthere !");
    expect(drawText("x".repeat(201))).toBe("x".repeat(201));
    expect(drawText("1\n2\n3\n4\n5\n6")).toBe("1\n2\n3\n4\n5\n6");
    expect(drawText("<b>hi</b>")).toBe("<b>hi</b>");
    expect(drawText("")).toBe("");
    expect(drawText(` ${BEL} `)).toBe("");
  });
});

describe("a text charm's fields (v1.62.0)", () => {
  const stored: TextInput = { text: "BRB", font: "sans", bold: true, color: "#abcdef", outline: true, align: "center" };

  it("readTextFields reads every field as stored when this build can use it", () => {
    const e = { text: " hi ", font: "mono", bold: false, color: "#ABCDEF", outline: false, align: "right", future: 1 };
    expect(readTextFields(e)).toEqual({
      text: "hi",
      font: "mono",
      bold: false,
      color: "#abcdef",
      outline: false,
      align: "right",
    });
    for (const font of ["display", "sans", "mono"] as const) expect(readTextFields({ font }).font).toBe(font);
    for (const align of ["left", "center", "right"] as const) expect(readTextFields({ align }).align).toBe(align);
  });

  it("readTextFields reads defaults for what it can't use, and never rewrites", () => {
    expect(readTextFields({})).toEqual({
      text: "",
      font: "sans",
      bold: true,
      color: "#ffffff",
      outline: true,
      align: "center",
    });
    // A stored string never reaches a style.
    for (const font of ["x; color: red", "serif", "Mono", "constructor", "toString", 2, null]) {
      expect(readTextFields({ font }).font, String(font)).toBe("sans");
    }
    for (const color of ["red", "#fff", "#ffffffff", "#gggggg", " #ffffff", "#ffffff;", 0xffffff]) {
      expect(readTextFields({ color }).color, String(color)).toBe("#ffffff");
    }
    for (const v of ["true", "false", 0, 1, null]) {
      expect(readTextFields({ bold: v }).bold, String(v)).toBe(true);
      expect(readTextFields({ outline: v }).outline, String(v)).toBe(true);
    }
    for (const align of ["centre", "justify", "Left", "hasOwnProperty", 1]) {
      expect(readTextFields({ align }).align, String(align)).toBe("center");
    }
    expect(readTextFields({ text: 5 }).text).toBe("");
    // Reading leaves the stored object as it was.
    const e = Object.freeze({ text: "a\r\nb", font: "x; color: red", color: "#ABCDEF", bold: "yes", align: 3 });
    const before = JSON.stringify(e);
    readTextFields(e);
    expect(JSON.stringify(e)).toBe(before);
    expect(e.color).toBe("#ABCDEF");
  });

  it("changedFields: only the fields that differ, the text compared normalized", () => {
    expect(changedFields(stored, { ...stored })).toEqual({});
    // A trailing space, or CR LF for LF, isn't a change.
    expect(changedFields(stored, { ...stored, text: " BRB \r\n" })).toEqual({});
    expect(changedFields({ ...stored, text: "a\nb" }, { ...stored, text: "a\r\nb" })).toEqual({});
    // The text given normalized.
    expect(changedFields(stored, { ...stored, text: " Back\r\nsoon\t!" })).toEqual({ text: "Back\nsoon !" });
    expect(changedFields(stored, { ...stored, text: "  " })).toEqual({ text: "" });
    expect(changedFields(stored, { ...stored, font: "mono", align: "left" })).toEqual({ font: "mono", align: "left" });
    expect(changedFields(stored, { ...stored, bold: false })).toEqual({ bold: false });
    expect(changedFields(stored, { ...stored, outline: false })).toEqual({ outline: false });
    expect(changedFields(stored, { ...stored, color: "#ffd60a" })).toEqual({ color: "#ffd60a" });
    // An untouched colour that isn't a swatch is absent, in any case.
    expect(changedFields(stored, { ...stored, color: "#ABCDEF" })).toEqual({});
    expect(changedFields(stored, { ...stored, color: "#FFD60A" })).toEqual({ color: "#ffd60a" });
    // All six, in the editor's order.
    const all = changedFields(stored, {
      text: "new",
      font: "display",
      bold: false,
      color: "#111111",
      outline: false,
      align: "right",
    });
    expect(Object.keys(all)).toEqual(["text", "font", "bold", "color", "outline", "align"]);
  });
});

describe("a text charm's natural box and fit (v1.62.0, §7.2)", () => {
  const widths =
    (table: Record<string, number>) =>
    (line: string): number => {
      const w = table[line];
      if (w === undefined) throw new Error(`no width for ${JSON.stringify(line)}`);
      return w;
    };

  it("textBox: the widest line and the lines at 1.15, with 0.12 em a side", () => {
    expect(textBox("BRB", widths({ BRB: 312.4 }))).toEqual({ nw: 337, nh: 139 });
    expect(textBox("ab\nabcd", widths({ ab: 200, abcd: 410.2 }))).toEqual({ nw: 435, nh: 254 });
    // A whole width isn't bumped up a pixel.
    expect(textBox("BRB", widths({ BRB: 313 }))).toEqual({ nw: 337, nh: 139 });
    // An empty line is 0 wide (never measured) and still a line.
    expect(textBox("a\n\nb", widths({ a: 50, b: 60 }))).toEqual({ nw: 84, nh: 369 });
    // One to five lines: 139, 254, 369, 484, 599 (1.15 · 100 isn't exact
    // in floating point).
    for (let n = 1; n <= MAX_LINES; n += 1) {
      const text = Array.from({ length: n }, () => "x").join("\n");
      expect(textBox(text, () => 10).nh, `${n} lines`).toBe(n * 115 + 24);
    }
    // A width that isn't a number counts as 0.
    expect(textBox("x", () => Number.NaN)).toEqual({ nw: 24, nh: 139 });
    expect(textBox("x", () => -5)).toEqual({ nw: 24, nh: 139 });
  });

  it.each([
    // [box w, box h, nw, nh, font size]
    [337, 139, 337, 139, 100],
    [168.5, 69.5, 337, 139, 50],
    [217.5, 127, 435, 254, 50],
    // A wide box: bound by its height, the text aligned across it.
    [337, 69.5, 337, 139, 50],
    // A tall box: bound by its width.
    [168.5, 139, 337, 139, 50],
    [0, 10, 337, 139, 0],
    [10, 0, 337, 139, 0],
    [-10, 10, 337, 139, 0],
    [Number.NaN, 10, 337, 139, 0],
    [Number.POSITIVE_INFINITY, 10, 337, 139, 0],
    [10, 10, 0, 139, 0],
    [10, 10, 337, Number.NaN, 0],
  ])("fitTextFont(%s, %s, %s, %s) is %s", (bw, bh, nw, nh, px) => {
    expect(fitTextFont(bw, bh, nw, nh)).toBeCloseTo(px, 9);
  });

  it("a box the size of the natural box fits at REF_PX", () => {
    const box = textBox("Now playing\nSomething", (line) => 47.3 * line.length);
    expect(fitTextFont(box.nw, box.nh, box.nw, box.nh)).toBe(REF_PX);
  });
});

describe("a text charm's colours and outline (v1.62.0, §7.3)", () => {
  // WCAG relative luminance, written out again here as the oracle.
  const channels = (hex: string): [number, number, number] =>
    [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255) as [number, number, number];
  const luminance = (hex: string): number => {
    const [r, g, b] = channels(hex).map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)) as [
      number,
      number,
      number,
    ];
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };

  // A port of OBS's chroma key distance (plugins/obs-filters:
  // chroma_key_filter_v2.effect `GetChromaDist`, chroma-key-filter.c's
  // vectors): Cb and Cr from the nonlinear sRGB values, distance in
  // (Cb, Cr). At OBS's defaults (similarity 400, smoothness 80, both
  // divided by 1000) a pixel's alpha is pow(saturate((d − 0.4) / 0.08),
  // 1.5), so it is fully opaque from 0.48.
  const CB = [-0.100644, -0.338572, 0.439216, 0.501961] as const;
  const CR = [0.439216, -0.398942, -0.040274, 0.501961] as const;
  const cbcr = (hex: string): [number, number] => {
    const [r, g, b] = channels(hex);
    return [CB[0] * r + CB[1] * g + CB[2] * b + CB[3], CR[0] * r + CR[1] * g + CR[2] * b + CR[3]];
  };
  const keyDistance = (hex: string, key: string): number => {
    const [a, b] = cbcr(hex);
    const [c, d] = cbcr(key);
    return Math.hypot(a - c, b - d);
  };
  const alpha = (d: number): number => Math.min(1, Math.max(0, (d - 0.4) / 0.08)) ** 1.5;
  const GREEN = "#00ff00";
  const MAGENTA = "#ff00ff";

  // [name, hex, L, outline, distance to Green, to Magenta]
  const TABLE: Array<[string, string, number, string, number, number]> = [
    ["White", "#ffffff", 1.0, "#000000", 0.523, 0.523],
    ["Yellow", "#ffd60a", 0.694, "#000000", 0.503, 0.766],
    ["Orange", "#ff9f0a", 0.461, "#000000", 0.589, 0.667],
    ["Red", "#ff4500", 0.255, "#000000", 0.745, 0.535],
    ["Blue", "#0a84ff", 0.238, "#000000", 0.622, 0.633],
    ["Sky", "#64d2ff", 0.56, "#000000", 0.502, 0.634],
    ["Grey", "#8e8e93", 0.272, "#000000", 0.528, 0.518],
    ["Black", "#111111", 0.006, "#ffffff", 0.523, 0.523],
  ];

  it("the table is the swatches, in order", () => {
    expect(TABLE.map(([name, hex]) => ({ name, hex }))).toEqual(COLOURS);
  });

  it("outline side by luminance: dark for light text, light for dark text", () => {
    for (const [name, hex, l, outline] of TABLE) {
      expect(luminance(hex), name).toBeCloseTo(l, 3);
      expect(outlineColor(hex), name).toBe(outline);
      expect(outlineColor(hex.toUpperCase()), name).toBe(outline);
    }
    // A colour this build can't read draws white, so its outline is dark.
    for (const junk of ["red", "#fff", ""]) expect(outlineColor(junk)).toBe("#000000");
    // Either side of the line.
    expect(outlineColor("#767676")).toBe("#000000"); // L 0.181
    expect(outlineColor("#757575")).toBe("#ffffff"); // L 0.178
  });

  it("textShadow is the timer face's three layers, dark or light", () => {
    // Stage.tsx's TIMER_FACE `text-shadow`.
    const dark = "0 0 0.06em #000, 0 0 0.12em rgba(0, 0, 0, 0.85), 0 0.04em 0.1em rgba(0, 0, 0, 0.6)";
    const light = "0 0 0.06em #fff, 0 0 0.12em rgba(255, 255, 255, 0.85), 0 0.04em 0.1em rgba(255, 255, 255, 0.6)";
    for (const [name, hex, , outline] of TABLE) {
      expect(textShadow(hex), name).toBe(outline === "#000000" ? dark : light);
    }
  });

  it("no swatch is near a key colour, in OBS's own key distance", () => {
    // The oracle: what the pre-review palette got wrong, and Divora's own
    // green, which OBS's Green preset keys.
    expect(keyDistance("#a78bfa", MAGENTA)).toBeCloseTo(0.401, 3);
    expect(alpha(keyDistance("#a78bfa", MAGENTA))).toBeLessThan(0.005);
    expect(keyDistance("#ff453a", MAGENTA)).toBeCloseTo(0.438, 3);
    expect(keyDistance("#ff3b30", MAGENTA)).toBeCloseTo(0.439, 3);
    expect(keyDistance("#00b140", GREEN)).toBeCloseTo(0.241, 3);
    expect(keyDistance(GREEN, GREEN)).toBe(0);
    for (const [name, hex, , , green, magenta] of TABLE) {
      expect(keyDistance(hex, GREEN), name).toBeCloseTo(green, 3);
      expect(keyDistance(hex, MAGENTA), name).toBeCloseTo(magenta, 3);
    }
    // Every swatch, both outline colours and the light's dot: at least 0.50
    // from both presets, so fully opaque at OBS's defaults.
    const drawn = [...COLOURS.map((c) => c.hex), "#000000", "#ffffff", LIGHT_RED];
    for (const hex of drawn) {
      for (const key of [GREEN, MAGENTA]) {
        expect(keyDistance(hex, key), `${hex} from ${key}`).toBeGreaterThanOrEqual(0.5);
        expect(alpha(keyDistance(hex, key)), `${hex} from ${key}`).toBe(1);
      }
    }
  });
});

describe("a text charm's label (v1.62.0)", () => {
  it("firstLine cuts at 40 and never splits a surrogate pair", () => {
    expect(firstLine(GAME.repeat(30), 40)).toBe(`${GAME.repeat(20)}…`);
    expect(firstLine(`a${GAME.repeat(20)}`, 40)).toBe(`a${GAME.repeat(19)}…`);
    expect(firstLine("x".repeat(40), 40)).toBe("x".repeat(40));
    expect(firstLine("x".repeat(41), 40)).toBe(`${"x".repeat(40)}…`);
    expect(firstLine("BRB\nback soon", 40)).toBe("BRB");
    expect(firstLine("hi   \nthere", 40)).toBe("hi");
    expect(firstLine(`${"x".repeat(39)} yz`, 40)).toBe(`${"x".repeat(39)}…`);
  });
});
