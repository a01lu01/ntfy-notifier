import test from "node:test";
import assert from "node:assert/strict";

import { createHistoryPoller, createLatestRefresh } from "../src/history-poller.js";

function harness({ active = true, revisions = [0], refresh } = {}) {
  let isActive = active;
  let revisionIndex = 0;
  const timers = new Map();
  const cancelled = [];
  const errors = [];
  let nextTimer = 1;
  let refreshCount = 0;

  const poller = createHistoryPoller({
    isActive: () => isActive,
    readRevision: async () => revisions[Math.min(revisionIndex++, revisions.length - 1)],
    refresh: async () => {
      refreshCount += 1;
      await refresh?.();
    },
    schedule: (callback, delay) => {
      const id = nextTimer++;
      timers.set(id, { callback, delay });
      return id;
    },
    cancel: (id) => {
      cancelled.push(id);
      timers.delete(id);
    },
    reportError: (error) => errors.push(error)
  });

  return {
    poller,
    timers,
    cancelled,
    errors,
    get refreshCount() { return refreshCount; },
    setActive(value) { isActive = value; }
  };
}

test("hidden or non-push pages do not query or schedule", async () => {
  const state = harness({ active: false });

  assert.equal(await state.poller.poll(), false);
  state.poller.start();

  assert.equal(state.refreshCount, 0);
  assert.equal(state.timers.size, 0);
});

test("first visible poll refreshes and unchanged revisions are deduplicated", async () => {
  const state = harness({ revisions: [7, 7, 8] });

  assert.equal(await state.poller.poll(), true);
  assert.equal(await state.poller.poll(), false);
  assert.equal(await state.poller.poll(), true);

  assert.equal(state.refreshCount, 2);
  assert.equal(Array.from(state.timers.values()).at(-1).delay, 4_000);
});

test("revision is captured before refresh so concurrent commits are observed next", async () => {
  let finishRefresh;
  const refreshBlocked = new Promise((resolve) => { finishRefresh = resolve; });
  const state = harness({
    revisions: [10, 11],
    refresh: () => refreshBlocked
  });

  const first = state.poller.poll();
  await Promise.resolve();
  assert.equal(await state.poller.poll(), false, "a second poll must not overlap refresh");
  finishRefresh();
  assert.equal(await first, true);

  assert.equal(await state.poller.poll(), true);
  assert.equal(state.refreshCount, 2);
});

test("leaving the page cancels the timer and becoming visible polls immediately", async () => {
  const state = harness({ revisions: [1, 2] });
  await state.poller.poll();
  assert.equal(state.timers.size, 1);

  state.setActive(false);
  state.poller.sync();
  assert.equal(state.timers.size, 0);
  assert.equal(state.cancelled.length, 1);

  state.setActive(true);
  state.poller.sync();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(state.refreshCount, 2);
});

test("poll errors are contained and retried on the regular schedule", async () => {
  let reads = 0;
  const timers = [];
  const expected = new Error("database busy");
  const poller = createHistoryPoller({
    isActive: () => true,
    readRevision: async () => {
      reads += 1;
      throw expected;
    },
    refresh: async () => assert.fail("refresh must not run after a failed revision read"),
    schedule: (callback, delay) => {
      timers.push({ callback, delay });
      return timers.length;
    },
    cancel: () => {},
    reportError: (error) => assert.equal(error, expected)
  });

  assert.equal(await poller.poll(), false);
  assert.equal(reads, 1);
  assert.equal(timers.at(-1).delay, 4_000);
});

test("a failed refresh does not acknowledge the revision and is retried", async () => {
  const expected = new Error("render failed");
  let attempts = 0;
  const state = harness({
    revisions: [12, 12],
    refresh: async () => {
      attempts += 1;
      if (attempts === 1) throw expected;
    }
  });

  assert.equal(await state.poller.poll(), false);
  assert.equal(state.errors.at(-1), expected);
  assert.equal(await state.poller.poll(), true);
  assert.equal(state.refreshCount, 2);
});

test("only the newest concurrent history response may render", async () => {
  const pending = [];
  const rendered = [];
  const refresh = createLatestRefresh({
    load: () => new Promise((resolve) => pending.push(resolve)),
    apply: (messages) => rendered.push(messages)
  });

  const older = refresh.run();
  const newer = refresh.run();
  pending[1](["new"]);
  assert.equal(await newer, true);
  pending[0](["old"]);
  assert.equal(await older, false);
  assert.deepEqual(rendered, [["new"]]);
});

test("invalidating a pending history read prevents it from rendering", async () => {
  let resolveRead;
  const rendered = [];
  const refresh = createLatestRefresh({
    load: () => new Promise((resolve) => { resolveRead = resolve; }),
    apply: (messages) => rendered.push(messages)
  });

  const pending = refresh.run();
  refresh.invalidate();
  resolveRead(["stale"]);

  assert.equal(await pending, false);
  assert.deepEqual(rendered, []);
});

test("a superseded poll is not acknowledged when the newer manual refresh fails", async () => {
  const pendingReads = [];
  const rendered = [];
  const refresh = createLatestRefresh({
    load: () => new Promise((resolve, reject) => pendingReads.push({ resolve, reject })),
    apply: (messages) => rendered.push(messages)
  });
  const poller = createHistoryPoller({
    isActive: () => true,
    readRevision: async () => 21,
    refresh: refresh.run,
    schedule: () => 1,
    cancel: () => {}
  });

  const polled = poller.poll();
  await Promise.resolve();
  const manual = refresh.run();
  const manualFailure = new Error("manual read failed");
  pendingReads[1].reject(manualFailure);
  await assert.rejects(manual, manualFailure);
  pendingReads[0].resolve(["stale"]);
  assert.equal(await polled, false);
  assert.deepEqual(rendered, []);

  const retry = poller.poll();
  await Promise.resolve();
  pendingReads[2].resolve(["current"]);
  assert.equal(await retry, true);
  assert.deepEqual(rendered, [["current"]]);
});
