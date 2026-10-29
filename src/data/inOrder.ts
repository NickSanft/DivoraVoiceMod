// v1.59.0: a feature's commands, sent one at a time.
//
// Tauri sends every invoke as its own fetch to http://ipc.localhost, and
// invokes in flight at the same time can reach Rust out of send order:
// measured in the real app with 40 sent at once. A sync command runs on the
// main thread when it reaches Rust, so "sync in the backend" orders nothing
// the page sends before the previous command answered. A queue from
// `inOrder` sends each command only once the one before it has answered or
// failed, so Rust applies them in the order the page sent them. Pure, so it
// unit-tests.

/** Queues `send`: it goes once every send queued before it has settled.
 *  The caller gets that send's own answer or error. */
export type InOrder = <T>(send: () => Promise<T>) => Promise<T>;

/** A new queue; the store keeps one per feature. A send queued while nothing
 *  is in flight goes out at once, in the caller's turn, so a command sent at
 *  the store's creation still goes before App's onMount sends anything. A
 *  failure never holds the rest back; a send that never settles holds back
 *  every one queued after it. The store queues only sync commands, which
 *  answer once the main thread runs them: one held up there holds up every
 *  sync command sent after it, queued or not. */
export function inOrder(): InOrder {
  let tail: Promise<void> = Promise.resolve();
  // Sends queued and not yet settled, the one in flight included.
  let pending = 0;
  const settled = (): void => {
    pending -= 1;
  };
  return <T>(send: () => Promise<T>): Promise<T> => {
    const run = pending === 0 ? now(send) : tail.then(send);
    pending += 1;
    tail = run.then(settled, settled);
    return run;
  };
}

/** `send()`, with a throw before it returned a promise as its rejection. */
function now<T>(send: () => Promise<T>): Promise<T> {
  try {
    return Promise.resolve(send());
  } catch (e) {
    return Promise.reject(e);
  }
}
