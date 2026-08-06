import test from "node:test";
import assert from "node:assert/strict";

import { cellValuesForOrder, moveInArray } from "../src/table-model.js";
import { columnDragOptions, isResizeHandleEvent } from "../src/table-drag.js";

test("moves an array item forward to the target index", () => {
  assert.deepEqual(moveInArray(["a", "b", "c"], 0, 2), ["b", "c", "a"]);
});

test("moves an array item backward to the target index", () => {
  assert.deepEqual(moveInArray(["a", "b", "c"], 2, 0), ["c", "a", "b"]);
});

test("returns the original order when indices are equal", () => {
  assert.deepEqual(moveInArray(["a", "b", "c"], 1, 1), ["a", "b", "c"]);
});

test("returns cell values in the persisted column order", () => {
  const message = { time: "10:00", title: "标题", message: "内容" };
  assert.deepEqual(
    cellValuesForOrder(["time", "message", "title"], message),
    ["10:00", "内容", "标题"]
  );
});

test("column drag uses fallback drag instead of native HTML5 drag", () => {
  assert.equal(columnDragOptions(() => {}).forceFallback, true);
});

test("resize handle events are excluded from column drag", () => {
  const resizeTarget = {
    closest: (selector) => (selector === ".resize-handle" ? {} : null),
  };
  const headerTarget = { closest: () => null };
  assert.equal(isResizeHandleEvent({ target: resizeTarget }), true);
  assert.equal(isResizeHandleEvent({ target: headerTarget }), false);
});
