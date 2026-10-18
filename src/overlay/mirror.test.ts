import { describe, expect, it } from "vitest";
import { emptyMirror, HOLD_MS, proposeEdit, receiveScene, renderedScene, type SceneMirror } from "./mirror";
import { applyOp, defaultScene, layoutScene, type OverlayScene, type SceneOp, type Size } from "./scene";
import type { ScenePush } from "./state";

const V: Size = { w: 480, h: 480 };
const ME = 3;

function sceneWith(x: number): OverlayScene {
  const s = defaultScene();
  return {
    ...s,
    elements: [
      ...s.elements,
      { id: "p", type: "image", x, y: 0.1, w: 0.2, h: 0.1, z: 1, src: "a1b2c3d4e5f60718.gif", nw: 2, nh: 1 },
    ],
  };
}

function push(scene: OverlayScene, revision: number, more: Partial<ScenePush> = {}): ScenePush {
  return { session: "s1", revision, scene, ack: null, expectViewport: null, ...more };
}

const moveP = (x: number): SceneOp => ({ kind: "place", id: "p", rect: { x, y: 48, w: 96, h: 48 }, viewport: V });
const xOf = (s: OverlayScene | null) => layoutScene(s!, V).find((r) => r.el.id === "p")!.rect.x;

function withEdits(m: SceneMirror, ops: SceneOp[]): SceneMirror {
  return ops.reduce((acc, op) => proposeEdit(acc, op, ME).mirror, m);
}

describe("the scene mirror", () => {
  it("draws nothing before the first push", () => {
    const m = emptyMirror();
    expect(m).toEqual({ session: null, revision: 0, server: null, pending: [], nextSeq: 0, held: null });
    expect(renderedScene(m, V, 0)).toBeNull();
  });

  it("draws the server scene with its pending edits on top", () => {
    const s = sceneWith(0.1);
    let m = receiveScene(emptyMirror(), push(s, 1), ME, 0, V);
    expect(renderedScene(m, V, 0)).toBe(s);
    const { mirror, edit } = proposeEdit(m, moveP(200), ME);
    m = mirror;
    expect(edit).toEqual({ instance: ME, seq: 0, op: moveP(200) });
    expect(renderedScene(m, V, 0)).toEqual(applyOp(s, moveP(200)));
    expect(xOf(renderedScene(m, V, 0))).toBeCloseTo(200, 3);
  });

  it("an ack for this window drops the edits it covers", () => {
    const s = sceneWith(0.1);
    let m = withEdits(receiveScene(emptyMirror(), push(s, 1), ME, 0, V), [moveP(100), moveP(150), moveP(200)]);
    expect(m.pending.map((p) => p.seq)).toEqual([0, 1, 2]);
    const s2 = applyOp(s, moveP(100));
    m = receiveScene(m, push(s2, 2, { ack: { instance: ME, seq: 0 } }), ME, 0, V);
    expect(m.pending.map((p) => p.seq)).toEqual([1, 2]);
    expect(xOf(renderedScene(m, V, 0))).toBeCloseTo(200, 3);
  });

  it("another window's ack is ignored", () => {
    const m = withEdits(receiveScene(emptyMirror(), push(sceneWith(0.1), 1), ME, 0, V), [moveP(100)]);
    const next = receiveScene(m, push(sceneWith(0.1), 2, { ack: { instance: ME + 1, seq: 0 } }), ME, 0, V);
    expect(next.pending.map((p) => p.seq)).toEqual([0]);
  });

  it("acks out of order never bring an edit back", () => {
    const five = [1, 2, 3, 4, 5].map(moveP);
    let m = withEdits(receiveScene(emptyMirror(), push(sceneWith(0.1), 1), ME, 0, V), five);
    m = receiveScene(m, push(sceneWith(0.1), 2, { ack: { instance: ME, seq: 3 } }), ME, 0, V);
    m = receiveScene(m, push(sceneWith(0.1), 3, { ack: { instance: ME, seq: 1 } }), ME, 0, V);
    expect(m.pending.map((p) => p.seq)).toEqual([4]);
  });

  it("a new session (main reloaded) adopts its scene, even at a lower revision, and drops every pending edit", () => {
    let m = withEdits(receiveScene(emptyMirror(), push(sceneWith(0.1), 9), ME, 0, V), [moveP(100), moveP(200)]);
    const fresh = sceneWith(0.5);
    m = receiveScene(m, { ...push(fresh, 1), session: "s2" }, ME, 0, V);
    expect(m.session).toBe("s2");
    expect(m.revision).toBe(1);
    expect(m.server).toBe(fresh);
    expect(m.pending).toEqual([]);
    // Edit numbers keep counting: main's ack names them.
    expect(m.nextSeq).toBe(2);
    expect(renderedScene(m, V, 0)).toBe(fresh);
  });

  it("within a session an older revision never replaces a newer one, but its ack still counts", () => {
    const five = sceneWith(0.3);
    const four = sceneWith(0.2);
    let m = withEdits(receiveScene(emptyMirror(), push(sceneWith(0.1), 1), ME, 0, V), [moveP(10), moveP(20)]);
    m = receiveScene(m, push(five, 5), ME, 0, V);
    m = receiveScene(m, push(four, 4, { ack: { instance: ME, seq: 1 } }), ME, 0, V);
    expect(m.revision).toBe(5);
    expect(m.server).toBe(five);
    expect(m.pending).toEqual([]);
  });

  it("an equal revision is taken (the ready re-push, a no-op edit's ack)", () => {
    const a = sceneWith(0.3);
    const b = sceneWith(0.4);
    let m = withEdits(receiveScene(emptyMirror(), push(a, 5), ME, 0, V), [moveP(10)]);
    m = receiveScene(m, push(b, 5, { ack: { instance: ME, seq: 0 } }), ME, 0, V);
    expect(m.server).toBe(b);
    expect(m.pending).toEqual([]);
  });

  it("keeps at most 64 pending edits, dropping the oldest", () => {
    const ops = Array.from({ length: 70 }, (_, i) => moveP(i));
    const m = withEdits(receiveScene(emptyMirror(), push(sceneWith(0.1), 1), ME, 0, V), ops);
    expect(m.pending).toHaveLength(64);
    expect(m.pending[0]!.seq).toBe(6);
    expect(m.nextSeq).toBe(70);
  });

  describe("expectViewport (a Fit change)", () => {
    const fitted = { w: 1920, h: 1080 };
    const old = sceneWith(0.1);
    const reframed = sceneWith(0.8);

    function heldMirror(now = 1000): SceneMirror {
      const m = receiveScene(emptyMirror(), push(old, 1), ME, 0, V);
      return receiveScene(m, push(reframed, 2, { expectViewport: fitted }), ME, now, V);
    }

    it("keeps drawing what was on screen until the window has the new size", () => {
      const m = heldMirror();
      expect(renderedScene(m, V, 1500)).toBe(old);
      expect(renderedScene(m, { w: 1000, h: 700 }, 1500)).toBe(old);
      // Within a pixel counts as arrived.
      expect(renderedScene(m, { w: 1919.2, h: 1080.9 }, 1500)).toBe(reframed);
      expect(renderedScene(m, fitted, 1500)).toBe(reframed);
    });

    it("for at most HOLD_MS", () => {
      const m = heldMirror(1000);
      expect(renderedScene(m, V, 1000 + HOLD_MS - 1)).toBe(old);
      expect(renderedScene(m, V, 1000 + HOLD_MS)).toBe(reframed);
      expect(HOLD_MS).toBe(1000);
    });

    it("holds the scene as drawn, pending edits included", () => {
      let m = withEdits(receiveScene(emptyMirror(), push(old, 1), ME, 0, V), [moveP(300)]);
      m = receiveScene(m, push(reframed, 2, { expectViewport: fitted }), ME, 0, V);
      expect(xOf(renderedScene(m, V, 10))).toBeCloseTo(300, 3);
    });

    it("a push without it ends the hold (a failed Fit change re-pushes the old scene)", () => {
      const m = receiveScene(heldMirror(), push(old, 3), ME, 1100, V);
      expect(m.held).toBeNull();
      expect(renderedScene(m, V, 1100)).toBe(old);
    });

    it("nothing to hold before the first scene", () => {
      const m = receiveScene(emptyMirror(), push(reframed, 1, { expectViewport: fitted }), ME, 0, V);
      expect(m.held).toBeNull();
      expect(renderedScene(m, V, 0)).toBe(reframed);
    });
  });

  it("a malformed push changes nothing", () => {
    const m = receiveScene(emptyMirror(), push(sceneWith(0.1), 1), ME, 0, V);
    for (const bad of [null, {}, { session: "s1", revision: 2, scene: null }]) {
      expect(receiveScene(m, bad as unknown as ScenePush, ME, 0, V)).toBe(m);
    }
  });
});
