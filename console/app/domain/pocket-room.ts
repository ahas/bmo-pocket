import { onBeforeUnmount, onMounted, ref } from "vue";

import type { Place } from "./pocket-place";
import type { Ref } from "vue";

// How the catalogue stands in the room the box under the toolbar has, never by the window's (docs/mcp-host.md, 「How
// the screen reads」): each of the three alone in turn, the open tool in the place of the tool list beside the
// namespaces, or the three side by side; and where the toolbar's way back leads in each.

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

/** The toolbar's way back in the catalogue: the place it leads to and the key of the words naming that place. */
export interface WayBack {
  to: Partial<Place>;
  names: string;
}

/**
 * Where the toolbar's way back leads: to the tool list where the open tool stands in its place, to the namespaces
 * where the tool list stands alone, and nowhere where every column the catalogue holds stands.
 */
export function wayBack(place: Place, room: CatalogueRoom): WayBack | null {
  if (place.view !== "catalogue") return null;
  if (place.tool !== null && room !== "three") return { to: { tool: null }, names: "pocket.label.tools" };
  if (place.namespace !== null && room === "one") return { to: { namespace: null, tool: null }, names: "pocket.label.namespaces" };

  return null;
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
