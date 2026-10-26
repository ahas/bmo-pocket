import { onBeforeUnmount, onMounted, ref } from "vue";

import type { Ref } from "vue";

// How the catalogue stands in the room its own box has, never by the window's (docs/mcp-host.md, 「How the screen
// reads」): each of the three alone in turn, the open tool in the place of the tool list beside the namespaces, or the
// three side by side.

export type CatalogueRoom = "one" | "two" | "three";

/** Below this width of its box the catalogue stands one column at a time. */
export const TWO_COLUMNS_FROM = 640;

/** From this width of its box the open tool stands beside the tool list rather than in its place. */
export const THREE_COLUMNS_FROM = 1100;

/** The columns a box of a width holds. */
export function roomOf(width: number): CatalogueRoom {
  if (width >= THREE_COLUMNS_FROM) return "three";

  return width >= TWO_COLUMNS_FROM ? "two" : "one";
}

/** The room an element's box holds, read again whenever the box changes size. */
export function useRoom(box: Readonly<Ref<HTMLElement | null>>): Ref<CatalogueRoom> {
  const room = ref<CatalogueRoom>("three");
  let watcher: ResizeObserver | null = null;

  onMounted(() => {
    const read = () => {
      if (box.value !== null) room.value = roomOf(box.value.getBoundingClientRect().width);
    };

    read();
    watcher = new ResizeObserver(read);

    if (box.value !== null) watcher.observe(box.value);
  });
  onBeforeUnmount(() => watcher?.disconnect());

  return room;
}
