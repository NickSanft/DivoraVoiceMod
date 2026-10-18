import { describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";
import type { ChainEntry } from "../types";
import { nodePositions, SpellCircle, SPELL_CIRCLE_PX } from "./SpellCircle";

const CENTER = 220;
const RADIUS = 158;
const EPS = 1e-6;

describe("SpellCircle.nodePositions", () => {
  it("returns an empty array for zero effects", () => {
    expect(nodePositions(0)).toEqual([]);
  });

  it("places a single node at the top of the orbit", () => {
    const [p] = nodePositions(1);
    expect(p).toBeDefined();
    expect(p!.x).toBeCloseTo(CENTER, 4);
    expect(p!.y).toBeCloseTo(CENTER - RADIUS, 4);
  });

  it("distributes 4 nodes at cardinal angles", () => {
    const nodes = nodePositions(4);
    expect(nodes).toHaveLength(4);
    // North, East, South, West
    expect(nodes[0]!.y).toBeCloseTo(CENTER - RADIUS, 4);
    expect(nodes[1]!.x).toBeCloseTo(CENTER + RADIUS, 4);
    expect(nodes[2]!.y).toBeCloseTo(CENTER + RADIUS, 4);
    expect(nodes[3]!.x).toBeCloseTo(CENTER - RADIUS, 4);
  });

  it("every node sits at the orbit radius", () => {
    const nodes = nodePositions(8);
    for (const n of nodes) {
      const dx = n.x - CENTER;
      const dy = n.y - CENTER;
      const r = Math.sqrt(dx * dx + dy * dy);
      expect(Math.abs(r - RADIUS)).toBeLessThan(EPS);
    }
  });

  it("angles are evenly spaced", () => {
    const nodes = nodePositions(5);
    const expectedStep = (2 * Math.PI) / 5;
    for (let i = 1; i < nodes.length; i++) {
      const diff = nodes[i]!.angle - nodes[i - 1]!.angle;
      expect(Math.abs(diff - expectedStep)).toBeLessThan(EPS);
    }
  });
});

// v1.55.0: the stream overlay draws the circle with `decorative`.
describe("SpellCircle decorative", () => {
  const chain: ChainEntry[] = [
    { id: "pitch", enabled: true, vals: {} },
    { id: "reverb", enabled: false, vals: {} },
  ];

  function draw(decorative: boolean) {
    const onSelect = vi.fn();
    const onToggle = vi.fn();
    const { container } = render(() => (
      <SpellCircle
        chain={chain}
        status="clean"
        motion={0}
        mystical={0.7}
        selected={null}
        onSelect={onSelect}
        onToggle={onToggle}
        decorative={decorative}
      />
    ));
    return { root: container.firstElementChild as HTMLElement, container, onSelect };
  }

  it("is SPELL_CIRCLE_PX square", () => {
    const { root } = draw(true);
    expect(SPELL_CIRCLE_PX).toBe(440);
    expect(root.style.width).toBe(`${SPELL_CIRCLE_PX}px`);
    expect(root.style.height).toBe(`${SPELL_CIRCLE_PX}px`);
  });

  it("leaves no title, takes no pointer and no focus", () => {
    const { root, container } = draw(true);
    expect(container.querySelectorAll("[title]")).toHaveLength(0);
    expect(root.style.pointerEvents).toBe("none");
    const nodes = container.querySelectorAll("button");
    expect(nodes).toHaveLength(2);
    for (const b of nodes) {
      expect(b.style.pointerEvents).toBe("none");
      expect(b.tabIndex).toBe(-1);
      expect(b.getAttribute("aria-hidden")).toBe("true");
    }
  });

  it("the Mixer's circle is unchanged: tooltips, focus and clicks", () => {
    const { root, container, onSelect } = draw(false);
    expect(root.style.pointerEvents).toBe("");
    const nodes = container.querySelectorAll("button");
    expect(nodes[0]!.getAttribute("title")).toMatch(/click to inspect/);
    expect(nodes[0]!.hasAttribute("tabindex")).toBe(false);
    expect(nodes[0]!.hasAttribute("aria-hidden")).toBe(false);
    expect(nodes[0]!.style.pointerEvents).toBe("");
    nodes[0]!.click();
    expect(onSelect).toHaveBeenCalledWith("pitch");
  });
});
