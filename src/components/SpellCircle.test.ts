import { describe, expect, it } from "vitest";
import { nodePositions } from "./SpellCircle";

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
