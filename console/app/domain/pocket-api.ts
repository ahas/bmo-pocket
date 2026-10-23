import { relayCall } from "bmo-ui/app/domain/relay";

import type { components } from "@bmo/pocket-client/schema";

// The tool host's reads as the screen calls them (docs/api.md, 「The console face」), each through the console's
// relay under the host's service name. Every read is a page of a list; the screen reads each list whole.

type Schemas = components["schemas"];

export type Namespace = Schemas["Namespace"];
export type Tool = Schemas["Tool"];
export type Schedule = Schemas["Schedule"];
export type StandingSession = Schemas["StandingSession"];
export type Server = Schemas["Server"];

interface Page<T> {
  items: T[];
  more: boolean;
}

// The name a console relays the host under, which its manifest gives.
const SERVICE = "pocket";

// The largest page the screen asks for.
const MOST = 200;

// Every page of a list, read until the last, each page asked after the key of the last member it answered.
async function whole<T, K extends string | number>(path: string, key: (member: T) => K): Promise<T[]> {
  const items: T[] = [];
  let after: K | null = null;

  for (;;) {
    const query: Record<string, string | number> = after === null ? { limit: MOST } : { limit: MOST, after };
    const page = await relayCall<Page<T>>(SERVICE, path, { query });

    items.push(...page.items);

    if (!page.more || page.items.length === 0) return items;

    after = key(page.items[page.items.length - 1]!);
  }
}

export function namespaces(): Promise<Namespace[]> {
  return whole<Namespace, string>("/namespaces", (namespace) => namespace.name);
}

export function tools(namespace: string): Promise<Tool[]> {
  return whole<Tool, string>(`/namespaces/${encodeURIComponent(namespace)}/tools`, (tool) => tool.name);
}

// A page of schedules never ends inside a run of equal times, so asking after the last one's time loses none.
export function schedules(): Promise<Schedule[]> {
  return whole<Schedule, number>("/schedules", (schedule) => schedule.nextAt);
}

export function sessions(): Promise<StandingSession[]> {
  return whole<StandingSession, number>("/tool-sessions", (session) => session.createdAt);
}

export function servers(): Promise<Server[]> {
  return whole<Server, string>("/servers", (server) => server.name);
}
