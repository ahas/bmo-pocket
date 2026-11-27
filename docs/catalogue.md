# The company catalogue

## What it is

- **The catalogue is where a person's item stands once they share it with their company,** one catalogue a
  company, kept at home beside the registry, so a receiver installs it whether or not the maker's PC is on. An item is
  one of three kinds (「Kinds」): a personal app, a skill or an MCP server. What is kept is the item's source, never a
  build: each receiver builds it from that source where it is built at all, and a reader or a head reads it before
  anything runs. The catalogue calls every item an app in its files and verbs, whatever its kind.
- **It lives under the data root, in the host's own folder:** `tool-host/catalogue/<company>/`, the company named by
  its unit's id:
  - `apps/<id>.json`, one record an app (「An app's record」);
  - `sources/<id>/<version>.json`, the source bundle each shared version was shared with (「A source bundle」);
  - `seen/<account>.json`, the moment a reader last looked at what was shared with them;
  - `catalogue.lock`, the lock every change to the company's catalogue is made under.
- **Only `bmo-pocket` reads or writes it,** through its `catalogue` verbs (`docs/api.md`, 「The CLI」), and the host's
  call path reads none of it: a received app reaches a session as a personal tool of its receiver's
  (`docs/registry.md`, 「Personal tools」), and nothing else. Whoever issues the sessions judges who may share, see,
  install, turn on and recall an app; the catalogue keeps what they decided.
- **A company's catalogue never names another company's account,** and no verb copies or moves an app from one
  company's catalogue to another's: a verb names the company it acts in, and an app's record stands in that company's
  folder alone. An app good for several companies is asked to be made a platform app (「A promotion」), which ships
  as a platform tool through `put`, never as a copy of the record.

## Kinds

- **The kind is read off the bundle's root as it is shared,** by the one file standing there that names it:
  `tool.toml`, a personal app; `SKILL.md`, a skill; `mcp.toml`, an MCP server. A bundle holding none of them, or more
  than one, is refused saying so. A version of an item shared as one kind is never shared as another.
- **A personal app** is as `docs/mcp-host.md`, 「Personal tools」, states: its `tool.toml` read as a personal tool's.
  Its text is the manifest's `[app]` name and description where it names one, else its first entry's title, or its
  name, and description. It asks for what its `[permissions]` names.
- **A skill** is instructions an agent's program reads: `SKILL.md`, opening on front matter, a YAML mapping between
  two lines of `---`, then its body; every other file of the bundle is a reference file of it. The front matter
  holds `name`, the item's name — a lower-case letter, then lower-case letters, digits and `-`, at most 64 — and
  `description`, the one string the program reads to know when to use it; and may hold `title` and `summary`, texts
  a screen shows it by (「Text in several languages」), and `language`. Its text is `title`, else its name, and
  `summary`, else its `description`. A skill asks for nothing: no network and no secret.
- **An MCP server** is a server an agent's program reaches through its receiver's runner: `mcp.toml`, its `[server]`
  table holding `name`, the item's name in a personal tool's alphabet; `title` and `description`, texts; `language`;
  and `transport`:
  - `local`, a server run on the receiver's PC, confined as a personal tool is: `command`, its words, run in its
    placed folder; and `build`, the shell line its source is built with, where it is built;
  - `remote`, a server reached at an address: `url`, an `http` or `https` address; `auth`, `none`, `bearer` or
    `header`; `header`, the header's name where `auth` is `header`; and `secret`, where `auth` is not `none`, the
    name of the secret each receiver holds in their own secret space. Its value is never in the item, so nothing of
    it is shared.

  A `[permissions]` table beside it reads as a personal tool's (`docs/api.md`, 「`[permissions]`」). What a version
  asks for is that table's, and for a remote server its address's host, with its port where it names one, among the
  network and its `secret` among the secrets. Every other key of either table is refused, naming it.
- **Any kind may carry a memory pack:** `memory.toml` at the bundle's root beside its kind file, which names no kind
  of its own. It holds one `[[memory]]` table a lesson the item leaves with whoever has it — 1 to 20, each `key`, a
  lower-case letter or digit, then lower-case letters, digits and `-`, at most 48, unique in the pack; `type`, `tip`
  or `check`; `text`, the instruction, 1 to 600 characters; `recall_condition`, the situation that calls for it, 1 to
  200 characters; and `importance`, 1 to 10, where it is given. Every other key is refused, naming it. The pack is
  instructions for agents written by the maker, so it is read in the review and shown to the receivers and the heads
  beside the permissions, and kept under the source hash like every file. Who issues the receivers' sessions leaves
  it in the brains of whoever holds a copy and takes it away when the copy goes (agency's `docs/person-nodes.md`,
  「Shared apps」).

## Text in several languages

- **An item's text may be written in one language or several, and none is required:** a share is never refused for a
  locale its text lacks. Each of an item's texts is a plain string or a table of locales out of `ko`, `en`, `ja` and
  `zh-CN` (`docs/api.md`, 「`[app]`」); a plain string is written in the language the item names, else in the maker's
  language, which the share names with `--language`, English where it names none.
- **The record keeps the text by locale:** `titles` and `descriptions` hold each locale the item's title or
  description is written in, and `title` and `description` the maker's language's, else the first the item holds.
  Each new version's share writes the locales its own text holds over what stood, taking their translation marks
  away, and every other locale stays as it stood.
- **`text` sets one locale's title and description on the record,** or clears it with `--clear`; `--machine` marks it
  as a translation wrote it, and a text set without it takes that mark away. The last locale an item holds is never
  cleared, and `title` and `description` follow the maker's language's text where it stands. Whoever issues the sessions decides who
  may set it and fills a locale by a translation when a person asks it to (agency's `docs/person-nodes.md`, 「Shared
  apps」); nothing here translates.

## An app's record

- **One JSON object an app, its keys in camelCase,** written whole beside its place and renamed over the old, under the
  catalogue's lock:
  - `id`, the app's name in the catalogue, `<maker>-<name>`, the maker's account and the app's name;
  - `kind`, `app`, `skill` or `mcp` (「Kinds」); a record that names none is an app's;
  - `key`, the app's key on the delegation chain (「The app on the delegation chain」);
  - `name`, the item's name, which a receiver's copy stands under; `title` and `description`, its text in the maker's
    language (「Text in several languages」);
  - `language`, the maker's language, the locale `title` and `description` are written in; `titles` and
    `descriptions`, the text by locale, each locale it holds; and `machine`, the locales whose text a translation
    wrote rather than a person;
  - `maker`, the account that shared it, and `owner`, the account answering for it now: the maker's until the maker
    leaves (「When the maker leaves」);
  - `unit`, the maker's home unit as they shared it, and `targets`, whom they shared it with, as the issuer judged
    them against the chain (「Targets」);
  - `versions`, every version shared, oldest first (「A version」);
  - `withdrawn`, the moment the owner withdrew it, or null;
  - `installs`, by account, each receiver's copy: `version`, `how` (`self` for one the receiver installed, `unit` for
    one a head's turning on installed), and `at`;
  - `promotion`, the request to make it a platform app, or null (「A promotion」);
  - `sharedAt`, the first share's moment, and `updatedAt`, the last change's.
- **An app's install count is the number of its `installs`.**

## A version

- **Each share places a version of the app's,** named by the version the maker's PC placed the app's build under, so
  the catalogue's version and the PC's are one name. A version shared once is never shared again under that name.
- **What it keeps:** `version`; `sourceHash` (「The source hash」); `permissions`, what the version asks for,
  `network` and `secrets` (「Kinds」); `memories`, its memory pack's lessons as shared, each `key`, `type`, `text`,
  `recallCondition` and `importance`, none where it carries no pack; `sharedAt`; `review`, the automatic review's report (「The
  review」); and `approval`: `required`, whether a head must approve it before anybody installs it, as the chain said
  for the maker when it was shared, and `by` and `at` once a head did.
- **The newest version is the app's:** a share of a new version notifies its receivers, and each copy moves onto it
  as its issuer decides (agency's `docs/person-nodes.md`, 「Shared apps」).

## A source bundle

- **One JSON object, `{"files": {<path>: <text>}}`:** every file of the app's folder as the maker's PC placed the
  version, by its path in the folder, its `.git` left out, each as UTF-8 text. A path is made of folder and file
  names alone, so none climbs out of the folder it is written into.
- **What a share checks before anything is kept,** each refusal naming what failed:
  - the bundle reads, and every path is inside its folder;
  - its kind reads (「Kinds」): an app's `tool.toml` as a personal tool's (`docs/mcp-host.md`, 「Personal tools」),
    every schema it names a JSON Schema; a skill's front matter; an MCP server's `mcp.toml`;
  - its memory pack reads, where it carries one (「Kinds」);
  - the version is not one of the app's already;
  - the app is not another company's, which no verb could name.
- **Nothing is built or run at home.** The bundle is kept as it came: a receiver's PC builds an app or an MCP server,
  and a skill is kept as it is by whoever issues the receiver's sessions.

## The source hash

- **The hash a share records is the receiver's check that what it builds is what was shared,** with no signature: who
  shared it is the account the record names, which the issuer proved.
- **It is the SHA-256, in lower-case hex, of the bundle's files in the byte order of their paths,** each written as
  its path's bytes, a zero byte, its content's length in bytes as decimal digits, a zero byte and its content's bytes.
  The same files give the same hash whatever order the bundle's object writes them in.
- **`source` reads it back checked:** the kept bundle's hash is reckoned again and the read refused where it is not
  the one recorded, so a bundle changed on disk is never handed out. The receiver's PC reckons it once more before it
  builds (agency's `docs/person-nodes.md`, 「Shared apps」).

## The app on the delegation chain

- **A shared app is an app as any other** (`docs/api.md`, 「An app on the delegation chain」): two kinds,
  `app.<key>.offered` and `app.<key>.on` carrying `personRaises`, declared by `share` with `declaredBy` `pocket`,
  so a head's switch, a person's switch and a lock reach it as they reach a platform app, and a session's issuer
  withholds it as it withholds any app.
- **Its key is `shared-<company>-<maker>-<name>`,** the name's `_` written `-`; where that is longer than an app key
  holds, the name is written as the first twelve hex digits of its SHA-256 instead.
- **Every kind stands on the chain the same way,** so a head turns a skill or an MCP server on and recalls it for a
  unit as they do an app.
- **Its defaults:** not offered and off. The issuer offers it to the maker's company alone, by the platform's rule
  naming the company, which it writes in the platform administrator's name as the platform link's one writer
  (agency's `docs/person-nodes.md`, 「Shared apps」), so no other company's people are offered it however their links
  stand; and the console shows a company's people and heads no other company's shared app, reading the company off
  the key; and it is on for a
  person only where a link says so — the person's own as they install it, or a unit's as its head turns it on there;
  and where a link above a person switched it on, their own link raises the flag and never lowers it
  (`personRaises`).
- **A receiver's copy names the key:** it is put as the receiver's personal tool with `put --owner --app <key>`, its
  pointer naming the app (`docs/registry.md`, 「Put a personal tool」), so the host withholds it as the chain says for
  the receiver. The maker's own app is the maker's personal tool and names no shared key.
- **A share with no account service named, or one it refuses, keeps nothing** and says why.

## Targets

- **Whom an app is shared with is a list of targets,** each one of `{"team": true}` (the maker's home unit and every
  unit under it), `{"heads": true}` (the heads of the units on the maker's line), `{"person": <account>}`,
  `{"group": <id>}` (a group of the delegation chain), `{"unit": <id>}` and `{"company": true}`. A reader sees the app
  where a target takes them in, and so do its maker and its owner.
- **Which targets a maker may name is the chain's,** as the issuer resolves it for them; the catalogue keeps the
  targets it was handed.

## The review

- **Every shared version, of every kind, is read by a reviewer** as its issuer has one read (agency's `docs/person-nodes.md`, 「Shared
  apps」), and the report is written onto the version with `review`: `state`, `pending` until written, `written` with
  the report's text, or `failed` with why none was written; and `at`.
- **The report is shown to the receivers and the heads** beside the permissions the version asks for; it decides
  nothing by itself.

## Withdrawal

- **`withdraw` marks the app withdrawn:** no new install is made of it from then, and every copy standing goes on as it
  is. A later share of a new version by its owner stands it again.
- **A recall is no withdrawal:** a head's or the administrator's recall is the chain's, the app's `offered` turned off
  and locked at their link, and the issuer takes the copies under it away (agency's `docs/person-nodes.md`, 「Shared
  apps」). The record stays as it is.

## When the maker leaves

- **The app stays in the catalogue, and its owner passes:** `pass` names the account that left and the one taking its
  apps over, its head as the issuer finds them, and every app of the company the leaver owns is the new owner's from
  then. The maker stays the maker.

## When an account or a company is deleted for good

- **An account's removal uninstalls its copies and forgets its seen marks** in every company's catalogue; the apps it
  made stay, as they stay when it leaves (`docs/api.md`, 「The console face」).
- **A company's removal takes its catalogue folder whole,** its records, its source bundles and its seen marks.

## A promotion

- **`promote` records the request to make the app a platform app,** by whom, when and with what note, at most one an
  app. `promotions` lists every request of every company for the administrator, who reviews the app's source and, where
  it is to ship, puts it as a platform tool with `put`; nothing of the request moves the record.
