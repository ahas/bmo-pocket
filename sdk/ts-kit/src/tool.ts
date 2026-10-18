// What a TypeScript tool reads its call with and answers it by, on the protocol the host speaks
// (docs/api.md, 「The call protocol」): the handed object off standard input, the entry it names answered
// on standard output, and another entry asked through the call's call-back socket. A tool's executable is
// its handlers by full entry name, handed to `answer`. The Python helper, `sdk/py-kit/src/bmo_tool.py`,
// answers the same way.

import { createConnection } from "node:net";

export const PROTOCOL = 1;

/** The session record a call is made under, as the host hands it. */
export interface Session {
  line: string;
  role?: string;
  capabilities: string[];
  shown?: string[];
  dataRoot: string;
  archive?: string;
  [field: string]: unknown;
}

/** One piece of an answer. */
export type Content = { type: "text"; text: string } | { type: "image"; data: string; mimeType: string };

/** The protocol's tool result. */
export interface Answer {
  content: Content[];
  isError: boolean;
  structuredContent?: Record<string, unknown>;
}

/** An answer of plain text. */
export function text(said: string): Answer {
  return { content: [{ type: "text", text: said }], isError: false };
}

/** A refusal, an answer marked as an error that the model reads as any other. */
export function refusal(said: string): Answer {
  return { content: [{ type: "text", text: said }], isError: true };
}

/** A tool that could not read what it answers out of: the reason, marked as the verbs mark it. */
export function failure(reason: unknown): Answer {
  return refusal(`ERROR: ${reason instanceof Error ? reason.message : String(reason)}`);
}

/** An object as an entry with an output schema answers it: the object, and the same object as its text. */
export function structured(object: Record<string, unknown>): Answer {
  return { content: [{ type: "text", text: JSON.stringify(object) }], isError: false, structuredContent: object };
}

/** The text of every text piece, joined, as a reader of text alone would read the answer. */
export function plain(answer: Answer): string {
  return answer.content.flatMap((piece) => (piece.type === "text" ? [piece.text] : [])).join("\n");
}

/** One JSON object asked on one line of a socket, and the one line it is answered with, read. */
export function askLine(path: string, asked: unknown): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const socket = createConnection({ path });
    let said = "";

    socket.setEncoding("utf8");
    socket.on("connect", () => socket.write(`${JSON.stringify(asked)}\n`));
    socket.on("data", (piece: string) => {
      said += piece;
      if (said.includes("\n")) socket.end();
    });
    socket.on("error", (why) => reject(new Error(`${path} could not be asked: ${why.message}`)));
    socket.on("close", () => {
      const line = said.split("\n")[0] ?? "";

      if (line.trim() === "") return reject(new Error(`${path} answered nothing`));
      try {
        resolve(JSON.parse(line));
      } catch (why) {
        reject(new Error(`the answer of ${path} does not read: ${(why as Error).message}`));
      }
    });
  });
}

/** One call as the host handed it: the entry, its arguments, the session record and the sockets. */
export class Call {
  readonly entry: string;
  readonly arguments: Record<string, unknown>;
  readonly session: Session;
  readonly callback: string | undefined;
  readonly shared: Record<string, string>;

  constructor(handed: Record<string, any>) {
    this.entry = handed.entry;
    this.arguments = handed.arguments ?? {};
    this.session = handed.session;
    this.callback = handed.callback;
    this.shared = handed.shared ?? {};
  }

  /** Another entry called under the same record, by its full name; answered with its tool result. */
  callBack(entry: string, args: Record<string, unknown> = {}): Promise<Answer> {
    return this.ask({ kind: "call", entry, arguments: args }) as Promise<Answer>;
  }

  /** Which entry the record holds claims this address, and the arguments it reads the page with. */
  claim(address: string): Promise<{ entry: string | null; arguments?: unknown }> {
    return this.ask({ kind: "claim", address }) as Promise<{ entry: string | null; arguments?: unknown }>;
  }

  private ask(asked: unknown): Promise<unknown> {
    if (this.callback === undefined) return Promise.reject(new Error("this call was handed no call-back socket"));

    return askLine(this.callback, asked);
  }
}

/** What one entry's executable does with its call. */
export type Handler = (call: Call) => Answer | Promise<Answer>;

/**
 * What the handed object asks, answered by the handlers: a call by its entry's handler, a claim with
 * none, and a resource request as an executable serving none answers it.
 */
export async function answerHanded(handlers: Record<string, Handler>, handed: Record<string, any>): Promise<unknown> {
  switch (handed.kind) {
    case "claim":
      return { arguments: null };
    case "resources_list":
      return { resources: [] };
    case "resources_read":
      return { error: "this executable serves no resources" };
    case "call":
      break;
    default:
      return failure("this executable answers calls alone");
  }

  const handler = handlers[handed.entry];

  if (handler === undefined) return failure(`this executable holds no entry ${handed.entry}`);
  try {
    return await handler(new Call(handed));
  } catch (why) {
    return failure(why);
  }
}

/** The handed object read out of its text, or why it does not read. */
export function readHanded(said: string): Record<string, any> | string {
  let handed: unknown;

  try {
    handed = JSON.parse(said);
  } catch (why) {
    return (why as Error).message;
  }
  if (typeof handed !== "object" || handed === null || typeof (handed as any).session !== "object") {
    return "it is no object holding a session";
  }
  if ((handed as any).protocol !== PROTOCOL) return `protocol ${(handed as any).protocol} is not ${PROTOCOL}`;

  return handed as Record<string, any>;
}

/**
 * The whole of a TypeScript tool's main: the call read off standard input and answered on standard
 * output; a call that does not read is said on standard error and ends unclean.
 */
export async function answer(handlers: Record<string, Handler>): Promise<void> {
  const handed = readHanded(await Bun.stdin.text());

  if (typeof handed === "string") {
    process.stderr.write(`the call does not read: ${handed}\n`);
    process.exit(1);
  }
  process.stdout.write(JSON.stringify(await answerHanded(handlers, handed)));
}
