---
name: bmo-pocket
description: Reach the tools a tool host holds for one tool session with the bmo-pocket MCP server. Use when a harness is handed a tool session token for a tool host, when looking for a tool among the namespaces that session may use, when calling one, or when making, testing, putting or debugging a tool the host serves.
---

# bmo-pocket

A tool host serves placed tools, grouped into namespaces, to the tool sessions it issues. A tool session's
record names the namespaces it may use, the capabilities it holds and the grants it may act under, and every
call is checked against it. The `bmo-pocket` MCP server holds one session's token and serves its tools to a
harness: every tool of the session's namespaces under a name of its own, and four tools that read the
catalogue and call any tool by name.

## Starting the server

Register the server with the harness as a stdio MCP server:

```
bmo-pocket mcp --socket /path/to/mcp-host.sock
```

The socket is the host's control socket; `BMO_MCP_HOST_SOCKET` names it as well, and the default path under the
user's runtime directory serves where neither does. The session's token goes in the `BMO_POCKET_TOKEN`
environment variable of the server, or `--token`, and every call is made with it. Whoever issued the session
holds the token; the server never issues one.

## The tools it serves

- **Each tool of the session's namespaces, as `<namespace>_<name>`,** with its own description, input schema,
  output schema where it declares one, and hints. Call it as any tool is called; the answer is the tool's own
  result. The list is read from the host at every listing, so a tool placed or taken away shows at the next.
- **`pocket_namespaces`** (`after?`, `limit?`): a page of the namespaces the session sees, each with what it says
  of itself (`facts`) and how many tools it holds there. `more` true means another page stands; ask again with
  `after` the last name answered.
- **`pocket_tools`** (`namespace`, `after?`, `limit?`): a page of one namespace's tools, each with its schemas,
  icon, hints and the grant it needs, if any.
- **`pocket_tool`** (`namespace`, `toolName`): one tool, its schemas with it.
- **`pocket_call`** (`namespace`, `name`, `input`): calls one tool by its namespace and name, answered with
  `result`, the tool's own result, and `spentMs`.

Two tools whose names would come out the same, or one whose name would be a fixed tool's, are not listed by
name. They stand in `pocket_tools`, and `pocket_call` reaches them.

## What to do first

1. Look at the listed tools: a tool named for its namespace is usually the one to call.
2. Where a tool is not listed, `pocket_namespaces` then `pocket_tools` find it, and `pocket_tool` shows what it
   takes.
3. Call it by its own name, or with `pocket_call`.

## Making a tool

A tool is made where the same kind of work keeps coming back and no tool the host serves does it. The tool host's
`docs/authoring.md` in the bmo-pocket repository is the guide: where a tool stands, its manifest field by field,
its schemas, the namespace, capabilities and grants that decide who sees it, the call protocol, and a working
example in each of Rust, TypeScript and Python under `examples/` to start from. Test it with its own check and
through a scratch host as the guide says, then:

- `bmo pocket put <folder | git address> --allow <capability>…` places it, served from the next call with no
  restart. Each `--allow` names one capability or grant the tool may ask for; the put refuses, naming them, those
  not allowed. Putting the same name again is the upgrade.
- `bmo pocket list` prints every registered tool, its version and where it came from; `bmo tool call
  <namespace>/<name> '<arguments>'` tries the live tool.
- `bmo pocket take <name>` takes it out again. Going back to an earlier version is putting the earlier source.

A service's own tool stands in that service's `tools/` folder and is put by its deployment, not by hand; a run
writing a tool's code commits it and puts nothing.

## When a tool answers an error

A refusal comes back as a tool error whose text is a JSON object of the host's `status`, `code` and `error`:

- `unauthorized` (401): the token is missing, wrong, revoked or past its expiry. Ask whoever issued the session
  for a new one.
- `forbidden` (403): the tool declares a grant the session does not hold; the sentence names it. The session's
  issuer decides grants.
- `not-found` (404): no tool of that namespace and name the session holds.
- `arguments-misfit` (422): the input does not fit the tool's schema; the sentence names each misfit and carries
  the schema. Call again with input that fits.
- `tool-failed` (502): the tool failed before it answered; the sentence names how.
- `bad-request` (400): the arguments of a fixed tool are not its input's shape.

A tool's own failure is not a refusal: it is the tool's result with `isError` true, as the tool wrote it.
