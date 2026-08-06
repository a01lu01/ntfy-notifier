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
