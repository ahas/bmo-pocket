//! A placed tool's manifest, `tool.toml` (docs/api.md, 「The manifest」): read here alone, by the host as it
//! reads its catalogue and by `bmo-pocket` as it builds and places a tool.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::tool::{Hints, Served};

/// The protocol version this side speaks; a manifest speaking another is left out rather than misread.
pub const PROTOCOL: u32 = 1;

/// The manifest's file name in a tool's folder.
pub const MANIFEST: &str = "tool.toml";

/// The folder under a tool's folder a deployment places its built tool in, the only one a reader of a
/// placed tool reads.
pub const PLACED: &str = ".placed";

/// How the host runs a tool: a process per call, or one MCP server per record.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
  #[default]
  Call,
  Server,
}

/// What the manifest says of the tool as a whole.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolPart {
  /// The folder's own name.
  pub name: String,
  pub protocol: u32,
  #[serde(default)]
  pub language: Option<String>,
  /// The command, run through `sh` in the tool's folder, that puts what the run command reads into
  /// `BMO_TOOL_OUT`; none where the folder is copied whole.
  #[serde(default)]
  pub build: Option<String>,
  /// Its tests, run through `sh` in the tool's folder.
  #[serde(default)]
  pub check: Option<String>,
  /// The run command as words, read in the placed folder.
  pub run: Vec<String>,
  #[serde(default)]
  pub mode: Mode,
  /// Read and given no effect: no call, helper or server has a span it must answer within.
  #[serde(default)]
  pub timeout: Option<u64>,
  /// The shared helpers a call of it is handed.
  #[serde(default)]
  pub shared: Vec<String>,
  /// The command, as words, run from the placed folder once the tool is placed.
  #[serde(default)]
  pub after_deploy: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HintsPart {
  #[serde(default)]
  pub read_only: bool,
  #[serde(default)]
  pub destructive: bool,
  #[serde(default)]
  pub idempotent: Option<bool>,
  #[serde(default)]
  pub open_world: bool,
}

impl From<HintsPart> for Hints {
  fn from(part: HintsPart) -> Self {
    Self {
      read_only: part.read_only,
      destructive: part.destructive,
      idempotent: part.idempotent,
      open_world: part.open_world,
    }
  }
}

/// The narrower form a bundle shows an entry in.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundlePart {
  pub shown: Vec<String>,
  #[serde(default)]
  pub required: Option<Vec<String>>,
}

/// The line an entry adds to its namespace's listing.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingPart {
  pub line: String,
  #[serde(default)]
  pub order: i64,
}

/// What the lines reading a namespace need of an entry beyond its schema.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactsPart {
  /// The verb a sending entry needs, a name its readers know.
  #[serde(default)]
  pub verb: Option<String>,
  /// The record a recording entry leaves.
  #[serde(default)]
  pub record: Option<String>,
  #[serde(default)]
  pub icon: Option<String>,
  /// The item it adds to each list of its namespace's facts, by the list's name; the host heads each with the
  /// entry's name.
  #[serde(default)]
  pub lists: BTreeMap<String, serde_json::Map<String, Value>>,
}

/// A time the host calls an entry by itself: a cron pattern, and the arguments each call is handed.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePart {
  pub cron: String,
  #[serde(default)]
  pub arguments: serde_json::Map<String, Value>,
}

impl SchedulePart {
  /// The pattern read, as the `croner` library reads a cron pattern; or why it does not read.
  pub fn pattern(&self) -> Result<croner::Cron, String> {
    self.cron.parse::<croner::Cron>().map_err(|why| why.to_string())
  }
}

/// The hosts whose pages an entry claims.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimsPart {
  pub hosts: Vec<String>,
}

/// One entry a session calls.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
  pub namespace: String,
  pub name: String,
  #[serde(default)]
  pub title: Option<String>,
  pub description: String,
  /// The Markdown file, by its path relative to the tool's folder and inside it, that says how the entry is
  /// called; the toolbook's `describe` answers it whole after the description.
  #[serde(default)]
  pub manual: Option<String>,
  /// The input schema's file, beside the manifest.
  pub input: String,
  /// The output schema's file, where it answers an object.
  #[serde(default)]
  pub output: Option<String>,
  #[serde(default)]
  pub hints: HintsPart,
  #[serde(default)]
  pub capabilities: Vec<String>,
  /// The resource a session's grants must hold for a call of it to run.
  #[serde(default)]
  pub grant: Option<String>,
  /// Read and given no effect, as the tool's own `timeout` is.
  #[serde(default)]
  pub timeout: Option<u64>,
  /// Read and given no effect: every call waits for its own answer or its caller going away.
  #[serde(default)]
  pub unbounded: bool,
  /// A call of it may wait on what stops the host, so it does not hold the host's stop: it is ended as the stop
  /// begins.
  #[serde(default)]
  pub ends_at_stop: bool,
  /// For the system alone: never shown to a session or callable by one.
  #[serde(default)]
  pub system: bool,
  /// Each bundle's form, by the bundle's capability.
  #[serde(default)]
  pub bundle: BTreeMap<String, BundlePart>,
  #[serde(default)]
  pub listing: Option<ListingPart>,
  #[serde(default)]
  pub facts: Option<FactsPart>,
  #[serde(default)]
  pub claims: Option<ClaimsPart>,
  /// The times the host calls it by itself.
  #[serde(default)]
  pub schedule: Vec<SchedulePart>,
  /// The input schema, read from its file.
  #[serde(skip)]
  pub input_schema: Value,
  #[serde(skip)]
  pub output_schema: Option<Value>,
  /// The manual's text, read from its file.
  #[serde(skip)]
  pub manual_text: Option<String>,
}

impl Entry {
  /// Its namespace and name, as the toolbook names it: `<namespace>/<name>`.
  pub fn full_name(&self) -> String {
    format!("{}/{}", self.namespace, self.name)
  }

  /// Whether it claims pages on this address's host: the host itself or one of its subdomains, as the host
  /// asks the entries claiming an address's host.
  pub fn claims(&self, address: &str) -> bool {
    let Some(host) = url::Url::parse(address).ok().and_then(|url| url.host_str().map(str::to_lowercase))
    else {
      return false;
    };

    self.claims.iter().flat_map(|claims| &claims.hosts).any(|claimed| {
      let claimed = claimed.to_lowercase();

      host == claimed || host.ends_with(&format!(".{claimed}"))
    })
  }
}

/// The resources a tool's executable serves.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcesPart {
  pub namespace: String,
  pub capabilities: Vec<String>,
}

/// What a shared helper's manifest says in place of entries.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperPart {
  /// What it holds for a record's calls.
  pub description: String,
}

/// The app a tool belongs to, where it is one a company, a unit or a person may have off (docs/mcp-host.md,
/// 「Apps」). A tool naming none is part of the system and is never withheld.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppPart {
  /// The app's key, which every tool and screen of the app names.
  pub key: String,
  pub name: String,
  pub description: String,
  #[serde(default)]
  pub icon: Option<String>,
  /// Whether it is offered where no link of the delegation chain says otherwise.
  #[serde(default = "yes")]
  pub offered: bool,
  /// Whether it is on where it is offered and no link says otherwise.
  #[serde(default = "yes")]
  pub on: bool,
}

fn yes() -> bool {
  true
}

/// What a personal tool's calls may reach beyond its own folders (docs/api.md, 「`[permissions]`」): the addresses
/// on the network and the owner's secrets by name, none of either by default. A platform tool's is read and given no
/// effect.
#[derive(Clone, Debug, Default, Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionsPart {
  /// The addresses a call may reach on the network; none reaches none.
  #[serde(default)]
  pub network: Vec<String>,
  /// The names of the owner's secrets a call is handed; none is handed none.
  #[serde(default)]
  pub secrets: Vec<String>,
}

/// The namespace every personal tool's entries stand in, which no platform tool's do (docs/mcp-host.md, 「Personal
/// tools」).
pub const PERSONAL_NAMESPACE: &str = "my";

// A declaration file of a screen-only app: the `[app]` table alone.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppWritten {
  app: AppPart,
}

impl AppPart {
  /// The app a declaration file's text holds, the `[app]` table alone, held to the manifest's rules for it; or why
  /// it does not read.
  pub fn parse(text: &str) -> Result<Self, String> {
    let written: AppWritten = toml::from_str(text).map_err(|why| why.message().to_owned())?;

    written.app.faults()?;
    Ok(written.app)
  }

  /// Why the app does not read: a key outside its alphabet or too long, or an empty name or description.
  pub fn faults(&self) -> Result<(), String> {
    if !app_key(&self.key) {
      return Err(format!("`{}` is no app's key", self.key));
    }

    if self.name.trim().is_empty() || self.description.trim().is_empty() {
      return Err(format!("the app `{}` has no name or no description", self.key));
    }

    Ok(())
  }
}

/// The longest key an app may have, so its settings' kind keys stay within the delegation chain's 64 characters.
pub const APP_KEY_MOST: usize = 50;

/// An app's key: a lower-case letter, then lower-case letters, digits and `-`, at most [`APP_KEY_MOST`] of them.
pub fn app_key(key: &str) -> bool {
  key.len() <= APP_KEY_MOST
    && key.chars().next().is_some_and(|first| first.is_ascii_lowercase())
    && key.chars().all(|one| one.is_ascii_lowercase() || one.is_ascii_digit() || one == '-')
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
  tool: ToolPart,
  #[serde(default)]
  entry: Vec<Entry>,
  #[serde(default)]
  resources: Option<ResourcesPart>,
  #[serde(default)]
  card: Option<toml::Table>,
  #[serde(default)]
  helper: Option<HelperPart>,
  #[serde(default)]
  app: Option<AppPart>,
  #[serde(default)]
  permissions: PermissionsPart,
}

/// One tool's manifest, read and held to its rules, its schemas read beside it.
#[derive(Clone, Debug)]
pub struct Manifest {
  pub tool: ToolPart,
  pub entries: Vec<Entry>,
  pub resources: Option<ResourcesPart>,
  /// The card master's declaration of the tool's node service, read by the card master alone.
  pub card: Option<toml::Table>,
  /// Present in a shared helper's manifest alone.
  pub helper: Option<HelperPart>,
  /// The app the tool belongs to, where it is one.
  pub app: Option<AppPart>,
  /// What a personal tool's calls may reach beyond its own folders.
  pub permissions: PermissionsPart,
}

/// A name a session calls an entry by: lower-case letters, digits and underscores.
pub fn entry_name(name: &str) -> bool {
  !name.is_empty() && name.chars().all(|one| one.is_ascii_lowercase() || one.is_ascii_digit() || one == '_')
}

/// A namespace, which is a path segment of the host: a lower-case letter, then letters, digits, `_` and `-`.
pub fn namespace_name(name: &str) -> bool {
  name.chars().next().is_some_and(|first| first.is_ascii_lowercase())
    && name
      .chars()
      .all(|one| one.is_ascii_lowercase() || one.is_ascii_digit() || one == '_' || one == '-')
}

/// A path inside a folder: relative, and made of folder and file names alone, so it never climbs out.
pub fn inside(path: &str) -> bool {
  !path.is_empty()
    && Path::new(path).components().all(|part| matches!(part, std::path::Component::Normal(_)))
}

// A file named beside the manifest, never one reached through another folder.
fn beside(file: &str) -> bool {
  !file.is_empty() && !file.contains('/') && file != "." && file != ".."
}

impl Manifest {
  /// The manifest standing in `dir`, whose folder is named `folder`, with its schemas read; or why it does
  /// not read.
  pub fn read(dir: &Path, folder: &str) -> Result<Self, String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).map_err(|why| format!("{MANIFEST}: {why}"))?;
    let mut manifest = Self::parse(&text, folder)?;

    for entry in &mut manifest.entries {
      let at = entry.full_name();

      entry.input_schema = schema(dir, &entry.input).map_err(|why| format!("{at}: {why}"))?;
      entry.output_schema = match &entry.output {
        Some(file) => Some(schema(dir, file).map_err(|why| format!("{at}: {why}"))?),
        None => None,
      };
      entry.manual_text = match &entry.manual {
        Some(file) => Some(manual(dir, file).map_err(|why| format!("{at}: {why}"))?),
        None => None,
      };
    }

    Ok(manifest)
  }

  /// A manifest's text held to its rules, its schemas not yet read.
  pub fn parse(text: &str, folder: &str) -> Result<Self, String> {
    let written: Written = toml::from_str(text).map_err(|why| format!("{MANIFEST}: {}", why.message()))?;
    let manifest = Self {
      tool: written.tool,
      entries: written.entry,
      resources: written.resources,
      card: written.card,
      helper: written.helper,
      app: written.app,
      permissions: written.permissions,
    };

    manifest.faults(folder)?;
    Ok(manifest)
  }

  fn faults(&self, folder: &str) -> Result<(), String> {
    let tool = &self.tool;

    if tool.protocol != PROTOCOL {
      return Err(format!("it speaks protocol {}, and this side speaks {PROTOCOL}", tool.protocol));
    }

    if tool.name != folder {
      return Err(format!("its name `{}` is not its folder's, `{folder}`", tool.name));
    }

    if tool.run.first().is_none_or(|word| word.trim().is_empty()) {
      return Err("it names no run command".to_owned());
    }

    if tool
      .after_deploy
      .as_ref()
      .is_some_and(|words| words.first().is_none_or(|word| word.trim().is_empty()))
    {
      return Err("its after-deploy command names nothing to run".to_owned());
    }

    if let Some(bad) = tool.shared.iter().find(|name| !beside(name)) {
      return Err(format!("`{bad}` is no shared helper's name"));
    }

    match (&self.helper, self.entries.is_empty()) {
      (Some(_), false) => return Err("a shared helper's manifest has entries".to_owned()),
      (None, true) if self.resources.is_none() => {
        return Err("it has neither an entry nor resources".to_owned());
      }
      _ => {}
    }

    if let Some(app) = &self.app {
      if self.helper.is_some() {
        return Err("a shared helper belongs to no app".to_owned());
      }

      app.faults()?;
    }

    let mut seen = BTreeSet::new();

    for entry in &self.entries {
      let at = entry.full_name();

      if !namespace_name(&entry.namespace) || entry.namespace == "toolbook" {
        return Err(format!("{at}: `{}` is no namespace's name", entry.namespace));
      }

      if !entry_name(&entry.name) {
        return Err(format!("{at}: `{}` is no entry's name", entry.name));
      }

      if !seen.insert(at.clone()) {
        return Err(format!("{at}: declared twice"));
      }

      if entry.description.trim().is_empty() {
        return Err(format!("{at}: no description"));
      }

      if entry.capabilities.is_empty() && !entry.system {
        return Err(format!("{at}: asks for no capability, so every session would be shown it"));
      }

      if entry.capabilities.iter().chain(entry.bundle.keys()).any(|one| one.trim().is_empty()) {
        return Err(format!("{at}: a capability with no name"));
      }

      if entry.grant.as_deref().is_some_and(|grant| grant.trim().is_empty()) {
        return Err(format!("{at}: a grant with no name"));
      }

      if let Some((schedule, why)) =
        entry.schedule.iter().find_map(|schedule| Some((schedule, schedule.pattern().err()?)))
      {
        return Err(format!("{at}: its schedule `{}` does not read: {why}", schedule.cron));
      }

      for file in std::iter::once(&entry.input).chain(&entry.output) {
        if !beside(file) {
          return Err(format!("{at}: its schema `{file}` does not stand beside the manifest"));
        }
      }

      if let Some(file) = entry.manual.as_deref().filter(|file| !inside(file)) {
        return Err(format!("{at}: its manual `{file}` does not stand inside the tool's folder"));
      }
    }

    if let Some(resources) = &self.resources
      && (!namespace_name(&resources.namespace) || resources.capabilities.is_empty())
    {
      return Err("its resources name no namespace or no capability".to_owned());
    }

    // A server is spoken to over MCP for its calls alone; claims and resources are asked a process at a time.
    if tool.mode == Mode::Server
      && (self.resources.is_some() || self.entries.iter().any(|entry| entry.claims.is_some()))
    {
      return Err("a server tool declares no claims and no resources".to_owned());
    }

    Ok(())
  }

  /// Why the manifest is no personal tool's (docs/mcp-host.md, 「Personal tools」): a personal tool is a tool called a
  /// process at a time on its owner's machine, its entries in the personal namespace, so it is no shared helper, no
  /// server, names no helper, serves no resources, claims no page, is called on no schedule and has no entry for the
  /// system alone or in a bundle's form; and it reaches neither the network nor a secret, which no runner gives a
  /// call yet.
  pub fn personal_faults(&self) -> Result<(), String> {
    if self.helper.is_some() {
      return Err("a personal tool is no shared helper".to_owned());
    }

    if self.tool.mode == Mode::Server || !self.tool.shared.is_empty() || self.resources.is_some() {
      return Err(
        "a personal tool is called a process at a time, with no server, helper or resources".to_owned(),
      );
    }

    for entry in &self.entries {
      let at = entry.full_name();

      if entry.namespace != PERSONAL_NAMESPACE {
        return Err(format!(
          "{at}: a personal tool's entries stand in the namespace `{PERSONAL_NAMESPACE}`"
        ));
      }

      if entry.system || !entry.bundle.is_empty() || entry.claims.is_some() || !entry.schedule.is_empty() {
        return Err(format!(
          "{at}: a personal tool's entry is for no system alone, has no bundle, claims no page and keeps no schedule"
        ));
      }
    }

    if !self.permissions.network.is_empty() || !self.permissions.secrets.is_empty() {
      return Err("it asks for the network or a secret, which no personal tool is given yet".to_owned());
    }

    Ok(())
  }

  /// The files of the folder the manifest itself names: the manifest, every schema and every manual, each by
  /// its path in the folder.
  pub fn named_files(&self) -> Vec<String> {
    let mut files = vec![MANIFEST.to_owned()];

    for entry in &self.entries {
      files.extend(
        std::iter::once(entry.input.clone())
          .chain(entry.output.clone())
          .chain(entry.manual.clone()),
      );
    }

    files.sort();
    files.dedup();
    files
  }

  /// What differs between its entries and the Rust tools its executable answers them with, each difference
  /// named: an entry no tool answers, a tool no entry declares, and a title, description, hint, capability,
  /// grant, bundle, system mark or schema the two say differently. A Rust tool's own test holds its manifest
  /// to its tools with it, so the folder's schema files are its input types' schemas.
  pub fn differences(&self, tools: &[&dyn Served]) -> Vec<String> {
    let mut differ = Vec::new();

    for entry in &self.entries {
      let at = entry.full_name();
      let Some(tool) = tools.iter().find(|tool| format!("{}/{}", tool.namespace(), tool.name()) == at)
      else {
        differ.push(format!("{at}: no tool answers it"));
        continue;
      };
      let capabilities: BTreeSet<&str> = tool.capabilities().iter().copied().collect();
      let bundle = tool.bundle().map(|bundle| {
        let part = BundlePart {
          shown: bundle.shown.iter().map(|one| (*one).to_owned()).collect(),
          required: bundle.required.map(|named| named.iter().map(|one| (*one).to_owned()).collect()),
        };

        (bundle.capability.to_owned(), part)
      });
      let said = [
        ("title", entry.title.as_deref() == tool.title()),
        ("description", entry.description == tool.description()),
        ("hints", Hints::from(entry.hints) == tool.hints()),
        (
          "capabilities",
          entry.capabilities.iter().map(String::as_str).collect::<BTreeSet<_>>() == capabilities,
        ),
        ("grant", entry.grant.as_deref() == tool.grant().map(|grant| grant.resource())),
        ("system", entry.system == tool.system()),
        (
          "bundle",
          entry.bundle.iter().map(|(name, part)| (name.clone(), part.clone())).collect::<Vec<_>>()
            == bundle.into_iter().collect::<Vec<_>>(),
        ),
        ("output schema", entry.output_schema == tool.output_schema()),
      ];

      for (what, same) in said {
        if !same {
          differ.push(format!("{at}: its {what} differs from its tool's"));
        }
      }

      if entry.input_schema != tool.input_schema() {
        differ.push(format!(
          "{at}: {} is not its input type's schema, which is {}",
          entry.input,
          tool.input_schema()
        ));
      }
    }

    for tool in tools {
      let at = format!("{}/{}", tool.namespace(), tool.name());

      if !self.entries.iter().any(|entry| entry.full_name() == at) {
        differ.push(format!("{at}: no entry declares it"));
      }
    }

    differ
  }

  /// Whether the tool is TypeScript, whose placement installs its packages.
  pub fn typescript(&self) -> bool {
    self
      .tool
      .language
      .as_deref()
      .is_some_and(|language| language.eq_ignore_ascii_case("typescript"))
  }
}

fn schema(dir: &Path, file: &str) -> Result<Value, String> {
  let text = std::fs::read_to_string(dir.join(file)).map_err(|why| format!("its schema {file}: {why}"))?;
  let read: Value = serde_json::from_str(&text).map_err(|why| format!("its schema {file}: {why}"))?;

  if !read.is_object() {
    return Err(format!("its schema {file} is not an object"));
  }

  Ok(read)
}

/// A manual's text, read from inside the folder: a link resolving outside it is refused as a path climbing out
/// would be.
pub fn manual(dir: &Path, file: &str) -> Result<String, String> {
  let unread = |why: std::io::Error| format!("its manual {file}: {why}");
  let folder = dir.canonicalize().map_err(unread)?;
  let found = dir.join(file).canonicalize().map_err(unread)?;

  if !found.starts_with(&folder) {
    return Err(format!("its manual {file} resolves outside the tool's folder"));
  }

  std::fs::read_to_string(&found).map_err(unread)
}

/// Every tool folder a tree's `tools/` holds, by its name: each folder whose name starts with neither `_` nor `.`,
/// in order of name.
pub fn tool_folders(tools: &Path) -> Vec<String> {
  let mut found: Vec<String> = std::fs::read_dir(tools)
    .into_iter()
    .flatten()
    .flatten()
    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
    .filter_map(|entry| entry.file_name().into_string().ok())
    .filter(|name| !name.starts_with(['_', '.']))
    .collect();

  found.sort();
  found
}
