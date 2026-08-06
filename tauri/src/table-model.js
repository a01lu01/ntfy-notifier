export function moveInArray(items, oldIndex, newIndex) {
  if (
    oldIndex < 0 ||
    newIndex < 0 ||
    oldIndex >= items.length ||
    newIndex >= items.length
  ) {
    return [...items];
  }
  const moved = [...items];
  const [item] = moved.splice(oldIndex, 1);
  moved.splice(newIndex, 0, item);
  return moved;
}

export function cellValuesForOrder(order, item) {
  return order.map((id) => item?.[id] ?? "");
}

export function resizeColumnBoundary(
  startCurrent,
  startNext,
  delta,
  minCurrent,
  minNext
) {
  if (startNext == null) {
    return {
      current: Math.max(minCurrent, startCurrent + delta),
      next: null,
    };
  }
  const minDelta = minCurrent - startCurrent;
  const maxDelta = startNext - minNext;
  const adjustedDelta = Math.min(Math.max(delta, minDelta), maxDelta);
  return {
    current: startCurrent + adjustedDelta,
    next: startNext - adjustedDelta,
  };
}
