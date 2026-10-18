// v1.55.0: the overlay's copy of the scene.
//
// Main owns the scene. The overlay keeps the last scene main pushed (the
// "server" scene) plus the edits it has sent and main hasn't acknowledged
// yet, and draws `pending.reduce(applyOp, server)`. So a dragged picture
// stays where it was dropped while its edit travels, and a push that
// crosses it (a Settings change, say) doesn't snap it back. Pure: the page
// holds the mirror in a signal.

import type { ScenePush, SceneEdit } from "./state";
import { applyOp, type OverlayScene, type SceneOp, type Size } from "./scene";

/** How long a Fit change may keep the previous scene on screen while the
 *  window is being resized. */
export const HOLD_MS = 1000;

/** The most unacknowledged edits kept; the oldest goes first. */
const MAX_PENDING = 64;

export interface PendingEdit {
  seq: number;
  op: SceneOp;
}

/** The scene drawn until the viewport reaches `expect`, or `until`. */
export interface HeldScene {
  scene: OverlayScene;
  expect: Size;
  until: number;
}

export interface SceneMirror {
  /** Main's page load the server scene came from; null before any push. */
  session: string | null;
  revision: number;
  server: OverlayScene | null;
  pending: PendingEdit[];
  /** The next edit's number. Never reset: main's ack names it. */
  nextSeq: number;
  held: HeldScene | null;
}

export function emptyMirror(): SceneMirror {
  return { session: null, revision: 0, server: null, pending: [], nextSeq: 0, held: null };
}

/** Take in an `overlay:scene` push.
 *
 *  - An ack for this window drops the edits it covers, always: a newer
 *    revision already holds every edit an older push acked.
 *  - A new session (main reloaded) adopts the scene and its revision, and
 *    drops every pending edit.
 *  - Within a session the scene is adopted only when its revision is at
 *    least the one held (equal: the ready re-push, and no-op acks).
 *  - `expectViewport` holds what was drawn before this push until the
 *    window has that size; a push without it ends any hold. */
export function receiveScene(
  m: SceneMirror,
  push: ScenePush,
  instance: number,
  now: number,
  viewport: Size,
): SceneMirror {
  if (typeof push !== "object" || push === null || typeof push.scene !== "object" || push.scene === null) return m;
  const shown = renderedScene(m, viewport, now);
  let pending = m.pending;
  const ack = push.ack;
  if (ack && ack.instance === instance) pending = pending.filter((p) => p.seq > ack.seq);
  let { session, revision, server } = m;
  if (push.session !== m.session) {
    session = push.session;
    revision = push.revision;
    server = push.scene;
    pending = [];
  } else if (push.revision >= m.revision) {
    revision = push.revision;
    server = push.scene;
  }
  const expect = push.expectViewport;
  const held = expect && shown ? { scene: shown, expect: { w: expect.w, h: expect.h }, until: now + HOLD_MS } : null;
  return { ...m, session, revision, server, pending, held };
}

/** Record an edit this window is sending, and number it. */
export function proposeEdit(
  m: SceneMirror,
  op: SceneOp,
  instance: number,
): { mirror: SceneMirror; edit: SceneEdit } {
  const seq = m.nextSeq;
  const pending = [...m.pending, { seq, op }].slice(-MAX_PENDING);
  return { mirror: { ...m, pending, nextSeq: seq + 1 }, edit: { instance, seq, op } };
}

/** What to draw: the held scene while a Fit change's resize hasn't arrived
 *  (within 1 px) and its time hasn't run out, else the server scene with
 *  the pending edits applied; null until the first push. */
export function renderedScene(m: SceneMirror, viewport: Size, now: number): OverlayScene | null {
  const h = m.held;
  if (h && now < h.until && (Math.abs(viewport.w - h.expect.w) > 1 || Math.abs(viewport.h - h.expect.h) > 1)) {
    return h.scene;
  }
  if (!m.server) return null;
  return m.pending.reduce((s, p) => applyOp(s, p.op), m.server);
}
