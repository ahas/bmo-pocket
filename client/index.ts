// The tool host's control socket as a TypeScript caller reaches it (docs/api.md, 「The OpenAPI document and the
// clients」): the types `openapi-typescript` generates from docs/openapi.json, and one call per route made through
// `openapi-fetch` over Bun's fetch on the Unix socket, each with the client's bearer token. A refusal of the host's
// is thrown as a `PocketError` carrying its status, code and sentence.

import { tmpdir } from "node:os";
import { join } from "node:path";

import createClient from "openapi-fetch";

import type { components, paths } from "./generated/openapi";

type Schemas = components["schemas"];

export type Health = Schemas["Health"];
export type Namespace = Schemas["Namespace"];
export type NamespacePage = Schemas["NamespacePage"];
export type Tool = Schemas["Tool"];
export type ToolPage = Schemas["ToolPage"];
export type ToolCallRequest = Schemas["ToolCallRequest"];
export type ToolCall = Schemas["ToolCall"];
export type Claim = Schemas["Claim"];
export type SessionRequest = Schemas["SessionRequest"];
export type ToolSession = Schemas["ToolSession"];
export type SchedulePage = Schemas["SchedulePage"];
export type Schedule = Schemas["Schedule"];
export type StandingSession = Schemas["StandingSession"];
export type SessionPage = Schemas["SessionPage"];
export type Server = Schemas["Server"];
export type ServerPage = Schemas["ServerPage"];
export type ErrorCode = Schemas["ErrorCode"];
export type WireError = Schemas["WireError"];

/** Where a page of a collection keyed by name begins, and how much of it is answered. */
export interface NameQuery {
  after?: string;
  limit?: number;
}

/** Where a page of a collection keyed by time begins, a time in epoch milliseconds, and how much of it is answered. */
export interface TimeQuery {
  after?: number;
  limit?: number;
}

/** A refusal or failure the host answered, in the common error shape. */
export class PocketError extends Error {
  readonly status: number;
  readonly code: ErrorCode | undefined;

  constructor(status: number, answered: unknown) {
    const error = answered as Partial<WireError> | undefined;

    super(`the host answered ${status}: ${error?.error ?? "no error answer"}${error?.code ? ` (${error.code})` : ""}`);
    this.name = "PocketError";
    this.status = status;
    this.code = error?.code;
  }
}

/** The control socket's path where nothing names another: `BMO_MCP_HOST_SOCKET`, or `bmo/mcp-host.sock` under the
 * user's runtime directory. */
export function defaultSocket(): string {
  return process.env.BMO_MCP_HOST_SOCKET ?? join(process.env.XDG_RUNTIME_DIR ?? tmpdir(), "bmo", "mcp-host.sock");
}

/** What a client is made with: the bearer token every call carries, the issuer's or a tool session's, and the
 * control socket, its default path where none is named. */
export interface PocketOptions {
  token: string;
  socket?: string;
}

// The route's answer, or its refusal thrown.
function answered<T>(result: { data?: T; error?: unknown; response: Response }): T {
  if (result.error !== undefined || !result.response.ok) {
    throw new PocketError(result.response.status, result.error);
  }

  return result.data as T;
}

/** The control socket, asked with one bearer token: one call per route. */
export function pocketClient(options: PocketOptions) {
  const socket = options.socket ?? defaultSocket();
  const client = createClient<paths>({
    baseUrl: "http://localhost",
    headers: { Authorization: `Bearer ${options.token}` },
    fetch: (request: Request) => fetch(request, { unix: socket }),
  });

  return {
    socket,

    /** `GET /health`: whether the host is ready, and its version. */
    async health(): Promise<Health> {
      return answered(await client.GET("/health"));
    },

    /** `GET /discipline`: the paragraph on finding and using tools that a session's system prompt carries. */
    async discipline(): Promise<string> {
      return answered(await client.GET("/discipline", { parseAs: "text" }));
    },

    /** `GET /namespaces`: a page of the namespaces the token sees, in order of name. */
    async namespaces(query: NameQuery = {}): Promise<NamespacePage> {
      return answered(await client.GET("/namespaces", { params: { query } }));
    },

    /** `GET /namespaces/{namespace}`: one namespace and its facts. */
    async namespace(namespace: string): Promise<Namespace> {
      return answered(await client.GET("/namespaces/{namespace}", { params: { path: { namespace } } }));
    },

    /** `GET /namespaces/{namespace}/tools`: a page of one namespace's tools the token sees, in order of name. */
    async tools(namespace: string, query: NameQuery = {}): Promise<ToolPage> {
      return answered(await client.GET("/namespaces/{namespace}/tools", { params: { path: { namespace }, query } }));
    },

    /** `GET /namespaces/{namespace}/tools/{toolName}`: one tool. */
    async tool(namespace: string, toolName: string): Promise<Tool> {
      const path = { namespace, toolName };

      return answered(await client.GET("/namespaces/{namespace}/tools/{toolName}", { params: { path } }));
    },

    /** `POST /tool-calls`: one tool called under the token's session. */
    async call(request: ToolCallRequest): Promise<ToolCall> {
      return answered(await client.POST("/tool-calls", { body: request }));
    },

    /** `POST /claims`: whether the token's session may have an address read, and by which entry. */
    async claim(resource: string): Promise<Claim> {
      return answered(await client.POST("/claims", { body: { resource } }));
    },

    /** `POST /tool-sessions`: a tool session issued, with the issuer's token. */
    async issue(request: SessionRequest): Promise<ToolSession> {
      return answered(await client.POST("/tool-sessions", { body: request }));
    },

    /** `GET /tool-sessions`: a page of the tool sessions standing, in order of when each was issued, with the
     * issuer's token. */
    async toolSessions(query: TimeQuery = {}): Promise<SessionPage> {
      return answered(await client.GET("/tool-sessions", { params: { query } }));
    },

    /** `GET /servers`: a page of the external servers registered, in order of name, with the issuer's token. */
    async servers(query: NameQuery = {}): Promise<ServerPage> {
      return answered(await client.GET("/servers", { params: { query } }));
    },

    /** `DELETE /tool-sessions/{token}`: a tool session revoked, with the issuer's token. */
    async revoke(token: string): Promise<void> {
      answered(await client.DELETE("/tool-sessions/{token}", { params: { path: { token } } }));
    },

    /** `GET /schedules`: a page of the schedules the token sees, in order of when each is next due. */
    async schedules(query: TimeQuery = {}): Promise<SchedulePage> {
      return answered(await client.GET("/schedules", { params: { query } }));
    },
  };
}

/** A client of the control socket, as `pocketClient` makes one. */
export type Pocket = ReturnType<typeof pocketClient>;
