# The registry

## What it is

- **The registry is the list of tools the host serves beside its tools root:** a standalone tool put by a person,
  and a service repository's own tool registered by that service's deployment. Both go through the same verb,
  `bmo-pocket put`, and the host reads both the same way (`docs/mcp-host.md`, 「The catalogue」).
- **It lives under the data root, in the host's own folder, `tool-host/`:**
  - `tool-host/registry/<name>.json`, one pointer per registered tool;
  - `tool-host/tools/<name>/<version>/`, each placed version of a tool;
  - `tool-host/registry.lock`, the lock every change to the registry is made under;
  - `tool-host/build/`, where a put clones and builds, and is cleared again.
- **Only the host and `bmo-pocket` read or write these files.** Another unit registers, takes and lists tools
  through `bmo-pocket` alone (`docs/api.md`, 「The CLI」), so the shape of a pointer is this repository's own.

## A pointer

- **One JSON object per tool, its keys in camelCase:** `name`, the tool's name, which is its manifest's; `version`,
  the placed version it points at; `dir`, that version's folder; `origin`, where it was put from (`path:<folder>`,
  `git:<address>` or `name:<name>`); `service`, the repository whose own tool it is, where a service registered it;
  and `putAt`, when, in epoch milliseconds. A pointer naming the time `put_at` reads as one naming it `putAt`, and
  is written as `putAt` at its next put.
- **A pointer is written whole or not at all:** written beside its place under a name of its own, then renamed
  over the old one, so the host never reads half of one.

## Versions

- **Each put places a version of its own,** named by the moment it was put and a random suffix, and never writes
  into a version already placed. An upgrade is a put that places a new version and moves the pointer onto it.
- **A version stays as long as anything holds it.** The host takes a version folder away only once it has left
  the catalogue and no call, server or shared helper started from it still runs, which it looks for each time it
  reads the catalogue, each time a call ends and on its sweep. So a take or an upgrade never breaks a call in
  flight: the call goes on in the version it started in, and the next call is answered by the catalogue as it
  then stands.
- **A record's server or shared helper already running goes on in the version it started in** until the record
  ends, as a helper placed anew serves the records started after it.
- **A version no pointer names when the host starts** is one no call can hold, and is taken away then, under the
  registry's lock, so a put placing a version that moment is never caught between its version and its pointer.

## Put

- **The source is one of three:**
  - **a local folder,** given by its path;
  - **a git address,** one holding `://`, starting `git@` or ending in `.git`, cloned whole into the build folder;
  - **a bare name,** made into the repository name `bmo-<name>` and put from the source template `--source` or
    `BMO_POCKET_SOURCE` names, `{repo}` in it standing for that repository name; a bare name with no template
    configured is refused. The template names whatever address the deployment's tools are kept at; no address
    stands in code.
- **The tool's folder is the one holding `tool.toml`:** the source itself, a folder or a repository's top.
- **What is checked before anything is placed,** each refusal naming what failed:
  - **the manifest reads,** and every schema it names is a JSON Schema (`docs/api.md`, 「The manifest」);
  - **its name is not another registration's of the other kind:** a tool put without `--service` never replaces
    one a service registered, a service never replaces one put without it, and one service never replaces
    another's;
  - **none of its entries' full names is held by another registered tool;**
  - **every capability and grant it asks for is allowed:** each capability its entries, their bundles and its
    resources name, and each grant its entries name, is printed, and the put is refused naming those no
    `--allow` names;
  - **its schedules are said:** each schedule an entry declares is printed with its pattern, since the host will
    call that entry by itself.
- **What is placed:**
  - with a `build` command, the command is run through `sh` in the tool's folder with `BMO_TOOL_OUT` naming the new
    version's staging folder and `BMO_CARGO_TARGET` the registry's own Cargo target folder, and the manifest and its
    schema files are copied beside what the build put there;
  - with no `build`, or with `--placed` for a folder a service's deployment already built, the folder is copied
    whole, its `.git` left out;
  - the staged version is read again as the host will read it, then renamed into place, and the pointer moved onto
    it, all under the registry's lock.
- **`after_deploy`,** where the manifest names one, is run from the placed version once its pointer stands; a
  failure is said and leaves the tool placed.

## Take and list

- **`take <name>`** removes the tool's pointer, under the lock; its versions are the host's to take away once
  nothing holds them. A tool a service registered is taken only with `--service` naming that service.
- **`list`** prints every pointer: its name, its version, where it was put from, the service it is registered
  for where one is, and its entries' full names; `--json` prints the pointers as they stand.

## What the host does with it

- **The host reads every pointer as it reads its catalogue,** beside the tools root: each pointer's version
  folder is a placed tool named by the pointer, or a shared helper where its manifest has a `[helper]` table. A
  pointer whose folder is gone, or whose manifest does not read, is left out and said in the log, as a placed
  folder is.
- **Shared helpers are found here alone:** a helper is registered like any tool, and a tool names the helpers it
  uses by their registered names.
- **The host watches the registry folder** as it watches the tools root, and reads the catalogue again once the
  changes settle: a put, an upgrade or a take is served from the next call, with no restart of the host or of a
  session, and changes no running session's prompt (`docs/toolbook.md`, 「Why」).
