export function isResizeHandleEvent(event) {
  return Boolean(event?.target?.closest?.(".resize-handle"));
}

export function columnDragOptions(onEnd) {
  return {
    animation: 150,
    draggable: "th",
    forceFallback: true,
    fallbackClass: "sortable-fallback",
    fallbackOnBody: true,
    filter: isResizeHandleEvent,
    ghostClass: "sortable-ghost",
    onEnd
  };
}
