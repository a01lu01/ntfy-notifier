const DEFAULT_INTERVAL_MS = 4_000;

/**
 * Polls the cross-process SQLite revision only while the mobile push page is visible.
 * A revision is acknowledged only after refreshing succeeds. The value being acknowledged was
 * read before refresh, so a message committed during refresh is still observed by the next poll.
 */
export function createHistoryPoller({
  isActive,
  readRevision,
  refresh,
  intervalMs = DEFAULT_INTERVAL_MS,
  schedule = (callback, delay) => setTimeout(callback, delay),
  cancel = (timer) => clearTimeout(timer),
  reportError = () => {}
}) {
  let stopped = false;
  let inFlight = false;
  let timer = null;
  let lastRevision = null;

  function clearTimer() {
    if (timer == null) return;
    cancel(timer);
    timer = null;
  }

  function arm() {
    clearTimer();
    if (stopped || inFlight || !isActive()) return;
    timer = schedule(() => {
      timer = null;
      void poll();
    }, intervalMs);
  }

  async function poll() {
    if (stopped || inFlight || !isActive()) {
      if (!isActive()) clearTimer();
      return false;
    }

    clearTimer();
    inFlight = true;
    try {
      const revision = await readRevision();
      const changed = lastRevision == null || !Object.is(revision, lastRevision);
      if (changed) {
        const refreshed = await refresh();
        if (refreshed === false) return false;
        lastRevision = revision;
      }
      return changed;
    } catch (error) {
      reportError(error);
      return false;
    } finally {
      inFlight = false;
      arm();
    }
  }

  function sync() {
    if (stopped) return;
    if (!isActive()) {
      clearTimer();
      return;
    }
    if (!inFlight) {
      clearTimer();
      void poll();
    }
  }

  function stop() {
    stopped = true;
    clearTimer();
  }

  return { poll, start: sync, stop, sync };
}

/**
 * Serializes rendering, rather than requests: slow older reads may finish, but only the most
 * recently requested snapshot is allowed to touch the DOM.
 */
export function createLatestRefresh({ load, apply }) {
  let generation = 0;

  async function run() {
    const request = ++generation;
    const value = await load();
    if (request !== generation) return false;
    apply(value);
    return true;
  }

  function invalidate() {
    generation += 1;
  }

  return { invalidate, run };
}
