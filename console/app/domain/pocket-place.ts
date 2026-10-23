// Where the screen stands, as its address's query holds it: the view open, and in the catalogue the namespace and
// the tool chosen, and the words the lists are narrowed by.

export const VIEWS = ["catalogue", "schedules", "sessions", "servers"] as const;

export type View = (typeof VIEWS)[number];

export interface Place {
  view: View;
  namespace: string | null;
  tool: string | null;
  q: string;
}

type Query = Record<string, unknown>;

function one(value: unknown): string | null {
  const first = Array.isArray(value) ? value[0] : value;

  return typeof first === "string" && first !== "" ? first : null;
}

/** The place an address's query names; the catalogue where it names no view the screen has. */
export function placeOf(query: Query): Place {
  const view = one(query.view);

  return {
    view: VIEWS.includes(view as View) ? (view as View) : "catalogue",
    namespace: one(query.namespace),
    tool: one(query.tool),
    q: one(query.q) ?? ""
  };
}

/** The query that names a place, with what it leaves empty left out. */
export function queryOf(place: Place): Record<string, string> {
  const query: Record<string, string> = {};

  if (place.view !== "catalogue") query.view = place.view;
  if (place.namespace !== null) query.namespace = place.namespace;
  if (place.tool !== null) query.tool = place.tool;
  if (place.q !== "") query.q = place.q;

  return query;
}

/** Whether every word of the search stands in one of the texts, ignoring case. */
export function matches(q: string, texts: readonly (string | null | undefined)[]): boolean {
  const words = q.toLowerCase().split(/\s+/).filter((word) => word !== "");
  const haystack = texts.filter((text): text is string => typeof text === "string").join("\n").toLowerCase();

  return words.every((word) => haystack.includes(word));
}

/** A description's first sentence, the line a list shows it by. */
export function firstSentence(text: string): string {
  const line = text.trim().split("\n")[0] ?? "";
  const end = line.search(/[.!?](\s|$)/);

  return end < 0 ? line : line.slice(0, end + 1);
}
