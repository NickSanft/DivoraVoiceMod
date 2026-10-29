import { describe, expect, it } from "vitest";
import { inOrder } from "./inOrder";

/** A send's answer, given by the test. */
function answer<T = string>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const settle = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

describe("inOrder", () => {
  it("a send queued while nothing is in flight goes out at once, in the caller's turn", () => {
    const queue = inOrder();
    const started: string[] = [];
    void queue(async () => {
      started.push("a");
    });
    expect(started).toEqual(["a"]);
  });

  it("each send goes only once the one before it has answered, and each caller gets its own answer", async () => {
    const queue = inOrder();
    const started: number[] = [];
    const answers = [answer(), answer(), answer()];
    const results = answers.map((a, i) =>
      queue(() => {
        started.push(i);
        return a.promise;
      }),
    );
    expect(started).toEqual([0]);
    await settle();
    expect(started).toEqual([0]);
    answers[0]!.resolve("a");
    await settle();
    expect(started).toEqual([0, 1]);
    answers[1]!.resolve("b");
    await settle();
    expect(started).toEqual([0, 1, 2]);
    answers[2]!.resolve("c");
    expect(await Promise.all(results)).toEqual(["a", "b", "c"]);
  });

  it("a failure never holds the rest back, and its caller gets the error", async () => {
    const queue = inOrder();
    const started: string[] = [];
    const first = answer();
    const failed = queue(() => {
      started.push("first");
      return first.promise;
    });
    const next = queue(async () => {
      started.push("next");
      return "ok";
    });
    await settle();
    expect(started).toEqual(["first"]);
    first.reject(new Error("refused"));
    await expect(failed).rejects.toThrow("refused");
    expect(await next).toBe("ok");
    expect(started).toEqual(["first", "next"]);
  });

  it("a send that throws before returning a promise is its caller's error, at once or queued", async () => {
    const queue = inOrder();
    const bad = (): Promise<string> => {
      throw new Error("bad args");
    };
    const atOnce = queue(bad);
    const queued = queue(bad);
    const after = queue(async () => "ok");
    await expect(atOnce).rejects.toThrow("bad args");
    await expect(queued).rejects.toThrow("bad args");
    expect(await after).toBe("ok");
  });

  it("once everything queued has settled, failures included, the next send goes out at once again", async () => {
    const queue = inOrder();
    const a = answer();
    const b = answer();
    const done = [queue(() => a.promise), queue(() => b.promise)];
    a.resolve("a");
    b.reject(new Error("refused"));
    await Promise.allSettled(done);
    await settle();
    const started: string[] = [];
    void queue(async () => {
      started.push("c");
    });
    expect(started).toEqual(["c"]);
  });

  it("keeps send order over a burst whose answers come back newest first", async () => {
    // What is in flight together is answered newest first: sent all at once,
    // the burst would arrive reversed.
    const queue = inOrder();
    const arrived: number[] = [];
    const inFlight: Array<() => void> = [];
    let peak = 0;
    const results = Array.from({ length: 20 }, (_, i) =>
      queue(
        () =>
          new Promise<number>((resolve) => {
            inFlight.push(() => {
              arrived.push(i);
              resolve(i);
            });
            peak = Math.max(peak, inFlight.length);
          }),
      ),
    );
    while (inFlight.length > 0) {
      inFlight.pop()!();
      await settle();
    }
    expect(arrived).toEqual(Array.from({ length: 20 }, (_, i) => i));
    expect(peak).toBe(1);
    expect(await Promise.all(results)).toEqual(arrived);
  });

  it("two queues don't wait on each other", () => {
    const one = inOrder();
    const two = inOrder();
    const started: string[] = [];
    void one(() => new Promise(() => {}));
    void two(async () => {
      started.push("two");
    });
    expect(started).toEqual(["two"]);
  });
});
