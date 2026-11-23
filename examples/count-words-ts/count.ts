// The example tool in TypeScript (the tool host's docs/authoring.md): its one entry, example/count_words_ts,
// answers how many words and characters a text holds. The host hands the call on standard input and reads the
// answer on standard output; the copy of the TypeScript kit beside this file, tool.ts, does both.

import { answer, structured, type Answer, type Call } from "./tool.ts";

/**
 * The entry's answer: the count as the object its output schema states, and the same object as its text. The
 * host checked the arguments against count.json before the call, so `text` stands and is a string.
 */
export function countWords(call: Call): Answer {
  const text = call.arguments.text as string;

  return structured({
    words: text.split(/\s+/u).filter((piece) => piece !== "").length,
    characters: [...text].length,
  });
}

if (import.meta.main) await answer({ "example/count_words_ts": countWords });
