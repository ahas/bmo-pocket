//! What `bmo-pocket` does to the registry (docs/registry.md): a tool put from a folder, a git address or a bare
//! name, checked, built or copied into a version of its own and pointed at; a registration taken; the registry
//! listed; an external server's registration checked and placed the same way; an owner's personal tool's
//! description copy placed, taken and listed in that owner's registry; and a screen-only app's settings declared
//! from its declaration file, which places nothing. Each verb writes what it did into `said`, a line each,
//! and answers why it refused where it did.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use bmo_tool::manifest::{AppPart, MANIFEST, Manifest};

use crate::chain::Chain;
use crate::external::{REGISTRATION, Registration};
use crate::placed::{platform_faults, schema_faults};
use crate::records::now_ms;
use crate::registry::{Pointer, Registry, owner_name};

/// One put as it is asked.
#[derive(Clone, Debug, Default)]
pub struct Put {
  /// A local folder, a git address or a bare tool name.
  pub tool: String,
  /// The capabilities and grants the tool may ask for.
  pub allow: Vec<String>,
  /// The repository whose own tool this is, where a service registers it.
  pub service: Option<String>,
  /// Whether the folder is one a deployment already built, placed whole.
  pub placed: bool,
  /// The source template a bare name is put from, `{repo}` in it standing for `bmo-<name>`.
  pub source: Option<String>,
  /// The account service a tool's app declares its settings on; a tool naming an app is refused without one.
  pub chain: Option<Chain>,
}

/// The placeholder of the source template.
const REPO: &str = "{repo}";

// A folder taken away as it is dropped: a clone, or a staged version that was never placed.
struct Scratch(Option<PathBuf>);

impl Drop for Scratch {
  fn drop(&mut self) {
    if let Some(dir) = &self.0 {
      let _ = fs::remove_dir_all(dir);
    }
  }
}

impl Scratch {
  fn keep(mut self) {
    self.0 = None;
  }
}

// A fresh name, for a version or a clone no other put holds.
fn fresh() -> String {
  format!("{}-{:06x}", now_ms(), rand::random::<u32>() & 0x00ff_ffff)
}

fn is_git_address(tool: &str) -> bool {
  tool.contains("://") || tool.starts_with("git@") || tool.ends_with(".git")
}

fn is_bare_name(tool: &str) -> bool {
  !tool.is_empty()
    && tool
      .chars()
      .all(|one| one.is_ascii_lowercase() || one.is_ascii_digit() || one == '-' || one == '_')
}

// The repository cloned into the build folder, where it is taken away again once the put is done.
fn clone(registry: &Registry, address: &str) -> Result<(PathBuf, Scratch), String> {
  fs::create_dir_all(registry.build())
    .map_err(|why| format!("the build folder could not be made: {why}"))?;

  let into = registry.build().join(format!("clone-{}", fresh()));
  let scratch = Scratch(Some(into.clone()));
  let ran = Command::new("git")
    .args(["clone", "--quiet", "--depth", "1", address])
    .arg(&into)
    .output()
    .map_err(|why| format!("git could not be run: {why}"))?;

  if !ran.status.success() {
    return Err(format!(
      "{address} could not be cloned: {}",
      String::from_utf8_lossy(&ran.stderr).trim()
    ));
  }

  Ok((into, scratch))
}

// The tool's folder, where it was put from, and the clone to take away after: a local folder, a git address, or a
// bare name made into its repository's address by the source template.
fn source(registry: &Registry, asked: &Put) -> Result<(PathBuf, String, Scratch), String> {
  let tool = asked.tool.trim();

  if Path::new(tool).is_dir() {
    let folder = fs::canonicalize(tool).map_err(|why| format!("{tool} could not be read: {why}"))?;

    return Ok((folder.clone(), format!("path:{}", folder.display()), Scratch(None)));
  }

  if is_git_address(tool) {
    let (folder, scratch) = clone(registry, tool)?;

    return Ok((folder, format!("git:{tool}"), scratch));
  }

  if !is_bare_name(tool) {
    return Err(format!("{tool} is no folder, git address or tool name"));
  }

  let Some(template) = asked.source.as_deref().filter(|template| !template.trim().is_empty()) else {
    return Err(format!(
      "{tool} is a bare name, which is put from the source template --source or BMO_POCKET_SOURCE names, and none \
       is configured"
    ));
  };

  if !template.contains(REPO) {
    return Err(format!("the source template {template} holds no {REPO}"));
  }

  let address = template.replace(REPO, &format!("bmo-{tool}"));
  let origin = format!("name:{tool}");

  if Path::new(&address).is_dir() {
    let folder = fs::canonicalize(&address).map_err(|why| format!("{address} could not be read: {why}"))?;

    return Ok((folder, origin, Scratch(None)));
  }

  let (folder, scratch) = clone(registry, &address)?;

  Ok((folder, origin, scratch))
}

// The tool's name, as its manifest names it.
fn manifest_name(folder: &Path) -> Result<String, String> {
  let text = fs::read_to_string(folder.join(MANIFEST))
    .map_err(|why| format!("{} holds no {MANIFEST}: {why}", folder.display()))?;
  let read: toml::Table = text.parse().map_err(|why| format!("its {MANIFEST} does not read: {why}"))?;

  read
    .get("tool")
    .and_then(|tool| tool.get("name"))
    .and_then(toml::Value::as_str)
    .filter(|name| is_bare_name(name))
    .map(str::to_owned)
    .ok_or_else(|| {
      format!("its {MANIFEST} names no tool by a name of lower-case letters, digits, `-` and `_`")
    })
}

/// What a tool asks for: every capability its entries, their bundles and its resources name, and every grant its
/// entries name.
pub fn asks(manifest: &Manifest) -> (BTreeSet<String>, BTreeSet<String>) {
  let capabilities = manifest
    .entries
    .iter()
    .flat_map(|entry| entry.capabilities.iter().chain(entry.bundle.keys()))
    .chain(manifest.resources.iter().flat_map(|resources| &resources.capabilities))
    .cloned()
    .collect();
  let grants = manifest.entries.iter().filter_map(|entry| entry.grant.clone()).collect();

  (capabilities, grants)
}

// Why the registration standing under the name may not be replaced by this put, where it may not.
fn replaces(standing: &Pointer, service: Option<&str>) -> Result<(), String> {
  match (standing.service.as_deref(), service) {
    (None, None) => Ok(()),
    (Some(had), Some(asked)) if had == asked => Ok(()),
    (Some(had), _) => Err(format!(
      "{} is registered for the service {had}, which alone replaces or takes it",
      standing.name
    )),
    (None, Some(_)) => Err(format!("{} was put by hand, and a service never replaces it", standing.name)),
  }
}

// The registered tool other than this one already holding one of its entries' full names, where one does.
fn clash(registry: &Registry, name: &str, manifest: &Manifest) -> Result<(), String> {
  let ours: BTreeSet<String> = manifest.entries.iter().map(|entry| entry.full_name()).collect();
  let (pointers, _) = registry.entries();

  for other in pointers.iter().filter(|other| other.name != name) {
    let Ok(theirs) = Manifest::read(&other.dir, &other.name) else {
      continue;
    };

    if let Some(held) =
      theirs.entries.iter().map(|entry| entry.full_name()).find(|full| ours.contains(full))
    {
      return Err(format!("{held} is the registered tool {}'s already", other.name));
    }
  }

  Ok(())
}

// A folder copied whole into another, its `.git` left out.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
  fs::create_dir_all(to)?;

  for entry in fs::read_dir(from)? {
    let entry = entry?;
    let name = entry.file_name();

    if name == ".git" {
      continue;
    }

    let (path, into) = (entry.path(), to.join(&name));

    if fs::metadata(&path)?.is_dir() {
      copy_tree(&path, &into)?;
    } else {
      fs::copy(&path, &into)?;
    }
  }

  Ok(())
}

// The last lines a command wrote on standard error, which a refusal carries.
fn last_lines(said: &[u8]) -> String {
  let text = String::from_utf8_lossy(said);
  let lines: Vec<&str> = text.trim().lines().collect();

  lines[lines.len().saturating_sub(5)..].join("\n")
}

// The version staged: the manifest's build run into it, the manifest and its schemas copied beside, or the folder
// copied whole.
fn stage(
  registry: &Registry,
  folder: &Path,
  manifest: &Manifest,
  placed: bool,
  into: &Path,
) -> Result<(), String> {
  match manifest.tool.build.as_deref().filter(|_| !placed) {
    Some(build) => {
      fs::create_dir_all(into).map_err(|why| format!("{} could not be made: {why}", into.display()))?;

      let ran = Command::new("sh")
        .args(["-c", build])
        .current_dir(folder)
        .env("BMO_TOOL_OUT", into)
        .env("BMO_CARGO_TARGET", registry.root.join("cargo-target"))
        .output()
        .map_err(|why| format!("its build could not be run: {why}"))?;

      if !ran.status.success() {
        return Err(format!("its build failed with {}: {}", ran.status, last_lines(&ran.stderr)));
      }

      for file in manifest.named_files() {
        let to = into.join(&file);

        if let Some(parent) = to.parent() {
          fs::create_dir_all(parent)
            .map_err(|why| format!("{} could not be made: {why}", parent.display()))?;
        }

        fs::copy(folder.join(&file), &to).map_err(|why| format!("{file} could not be copied: {why}"))?;
      }

      Ok(())
    }
    None => {
      copy_tree(folder, into).map_err(|why| format!("{} could not be copied: {why}", folder.display()))
    }
  }
}

/// A tool put: checked, staged as a version of its own, read again as the host reads it, then placed and pointed
/// at under the registry's lock; its `after_deploy` run once it stands.
pub fn put(registry: &Registry, asked: &Put, said: &mut Vec<String>) -> Result<(), String> {
  let (folder, origin, _clone) = source(registry, asked)?;
  let name = manifest_name(&folder)?;
  let manifest =
    Manifest::read(&folder, &name).map_err(|why| format!("{name}: its manifest does not read: {why}"))?;

  if let Some(why) = schema_faults(&manifest).or_else(|| platform_faults(&manifest)) {
    return Err(format!("{name}: {why}"));
  }

  let (capabilities, grants) = asks(&manifest);
  let allowed: BTreeSet<&str> = asked.allow.iter().map(String::as_str).collect();

  said.extend(capabilities.iter().map(|one| format!("{name} asks for the capability {one}")));
  said.extend(grants.iter().map(|one| format!("{name} asks for the grant {one}")));
  said.extend(manifest.entries.iter().flat_map(|entry| {
    entry
      .schedule
      .iter()
      .map(|schedule| format!("{name} has the host call {} at `{}`", entry.full_name(), schedule.cron))
  }));

  let missing: Vec<&String> = capabilities
    .iter()
    .chain(&grants)
    .filter(|one| !allowed.contains(one.as_str()))
    .collect();

  if !missing.is_empty() {
    let named: Vec<&str> = missing.iter().map(|one| one.as_str()).collect();

    return Err(format!("{name} asks for what no --allow names: {}", named.join(", ")));
  }

  if let Some(standing) = registry.get(&name) {
    replaces(&standing, asked.service.as_deref())?;
  }

  clash(registry, &name, &manifest)?;

  // The app's settings stand on the delegation chain before its tool is served, so no session is ever handed an
  // app whose settings nobody can resolve (docs/api.md, 「An app on the delegation chain」).
  if let Some(app) = &manifest.app {
    let Some(chain) = &asked.chain else {
      return Err(format!(
        "{name} belongs to the app {}, whose settings could not be declared: no account service is named",
        app.key
      ));
    };

    said.extend(chain.declare(app).map_err(|why| format!("{name}: {why}"))?);
  }

  let version = fresh();
  let staged = registry.versions().join(&name).join(format!(".staging-{version}"));
  let staging = Scratch(Some(staged.clone()));

  stage(registry, &folder, &manifest, asked.placed, &staged)?;
  Manifest::read(&staged, &name)
    .map_err(|why| format!("{name}: its placed version does not read: {why}"))?;

  let placed = registry.version_dir(&name, &version);
  let locked = registry.lock().map_err(|why| format!("the registry could not be locked: {why}"))?;
  // Read again under the lock: another put may have registered the name or an entry since.
  let standing = registry.get(&name);

  if let Some(standing) = &standing {
    replaces(standing, asked.service.as_deref())?;
  }

  clash(registry, &name, &manifest)?;
  fs::rename(&staged, &placed).map_err(|why| format!("{} could not be placed: {why}", placed.display()))?;
  staging.keep();

  let pointer = Pointer {
    name: name.clone(),
    version: version.clone(),
    dir: placed.clone(),
    origin: origin.clone(),
    service: asked.service.clone(),
    app: None,
    owner: None,
    put_at: now_ms(),
  };

  if let Err(why) = registry.point(&pointer) {
    let _ = fs::remove_dir_all(&placed);

    return Err(format!("the registry could not point at {name}: {why}"));
  }

  drop(locked);

  said.push(match standing {
    Some(was) => format!("upgraded {name} from {} to {version}", was.version),
    None => format!("put {name} {version} from {origin}"),
  });

  if let Some(words) = &manifest.tool.after_deploy {
    let ran = Command::new(&words[0]).args(&words[1..]).current_dir(&placed).output();

    said.push(match ran {
      Ok(done) if done.status.success() => format!("{name}: after_deploy ran"),
      Ok(done) => format!("{name}: after_deploy failed with {}: {}", done.status, last_lines(&done.stderr)),
      Err(why) => format!("{name}: after_deploy could not be run: {why}"),
    });
  }

  Ok(())
}

/// An external server registered by its registration file: checked, its registration and the manuals it names
/// staged as a version of their own and read again as the host reads them, then placed and pointed at under the
/// registry's lock (docs/registry.md, 「Put an external server」).
pub fn register_server(registry: &Registry, asked: &Put, said: &mut Vec<String>) -> Result<(), String> {
  let file = fs::canonicalize(asked.tool.trim())
    .map_err(|why| format!("{} could not be read: {why}", asked.tool.trim()))?;
  let registration =
    Registration::read_file(&file).map_err(|why| format!("its registration does not read: {why}"))?;
  let name = registration.name.clone();
  let origin = format!("path:{}", file.display());
  let allowed: BTreeSet<&str> = asked.allow.iter().map(String::as_str).collect();
  let capabilities: BTreeSet<&String> = registration.capabilities.iter().collect();

  said.extend(capabilities.iter().map(|one| format!("{name} asks for the capability {one}")));

  let missing: Vec<&str> = capabilities
    .iter()
    .map(|one| one.as_str())
    .filter(|one| !allowed.contains(one))
    .collect();

  if !missing.is_empty() {
    return Err(format!("{name} asks for what no --allow names: {}", missing.join(", ")));
  }

  if let Some(standing) = registry.get(&name) {
    replaces(&standing, asked.service.as_deref())?;
  }

  let version = fresh();
  let staged = registry.versions().join(&name).join(format!(".staging-{version}"));
  let staging = Scratch(Some(staged.clone()));
  let folder = file.parent().unwrap_or(Path::new("/"));

  for named in std::iter::once(REGISTRATION.to_owned()).chain(registration.named_files()) {
    let (from, to) = match named.as_str() {
      REGISTRATION => (file.clone(), staged.join(REGISTRATION)),
      _ => (folder.join(&named), staged.join(&named)),
    };

    if let Some(parent) = to.parent() {
      fs::create_dir_all(parent).map_err(|why| format!("{} could not be made: {why}", parent.display()))?;
    }

    fs::copy(&from, &to).map_err(|why| format!("{named} could not be copied: {why}"))?;
  }

  Registration::read(&staged).map_err(|why| format!("{name}: its placed version does not read: {why}"))?;

  let placed = registry.version_dir(&name, &version);
  let locked = registry.lock().map_err(|why| format!("the registry could not be locked: {why}"))?;
  // Read again under the lock: another put may have registered the name since.
  let standing = registry.get(&name);

  if let Some(standing) = &standing {
    replaces(standing, asked.service.as_deref())?;
  }

  fs::rename(&staged, &placed).map_err(|why| format!("{} could not be placed: {why}", placed.display()))?;
  staging.keep();

  let pointer = Pointer {
    name: name.clone(),
    version: version.clone(),
    dir: placed.clone(),
    origin: origin.clone(),
    service: asked.service.clone(),
    app: None,
    owner: None,
    put_at: now_ms(),
  };

  if let Err(why) = registry.point(&pointer) {
    let _ = fs::remove_dir_all(&placed);

    return Err(format!("the registry could not point at {name}: {why}"));
  }

  drop(locked);

  said.push(match standing {
    Some(was) => format!("upgraded {name} from {} to {version}", was.version),
    None => format!("registered the external server {name} {version} from {origin}"),
  });

  Ok(())
}

/// A screen-only app's two settings declared on the delegation chain from its declaration file, the `[app]` table
/// alone (docs/api.md, 「An app on the delegation chain」); nothing is placed in the registry.
pub fn app(file: &Path, chain: Option<&Chain>, said: &mut Vec<String>) -> Result<(), String> {
  let shown = file.display();
  let text = fs::read_to_string(file).map_err(|why| format!("{shown}: {why}"))?;
  let app = AppPart::parse(&text).map_err(|why| format!("{shown}: {why}"))?;
  let Some(chain) = chain else {
    return Err(format!(
      "the settings of the app {} could not be declared: no account service is named",
      app.key
    ));
  };

  said.extend(chain.declare(&app).map_err(|why| format!("the app {}: {why}", app.key))?);
  Ok(())
}

/// A registration taken: its pointer removed under the registry's lock, its versions left for the host to take
/// away once nothing holds them.
pub fn take(
  registry: &Registry,
  name: &str,
  service: Option<&str>,
  said: &mut Vec<String>,
) -> Result<(), String> {
  let _locked = registry.lock().map_err(|why| format!("the registry could not be locked: {why}"))?;
  let Some(standing) = registry.get(name) else {
    return Err(format!("no tool {name} is registered"));
  };

  match (standing.service.as_deref(), service) {
    (None, None) => {}
    (Some(had), Some(asked)) if had == asked => {}
    (Some(had), _) => {
      return Err(format!("{name} is registered for the service {had}; take it with --service {had}"));
    }
    (None, Some(asked)) => {
      return Err(format!("{name} was put by hand, not registered for the service {asked}"));
    }
  }

  registry
    .unpoint(name)
    .map_err(|why| format!("{name}'s pointer could not be taken away: {why}"))?;
  said.push(format!("took {name} {}", standing.version));

  Ok(())
}

/// Every registration, a line each, or the pointers as they stand where `json` asks.
pub fn list(registry: &Registry, json: bool, said: &mut Vec<String>) -> Result<(), String> {
  let (pointers, faults) = registry.entries();

  if json {
    said.push(serde_json::to_string_pretty(&pointers).map_err(|why| why.to_string())?);
    return Ok(());
  }

  for pointer in &pointers {
    let entries = if pointer.dir.join(REGISTRATION).exists() {
      Registration::read(&pointer.dir).map_or_else(
        |why| format!("its registration does not read: {why}"),
        |server| format!("the external server {} under {}/", server.address(), server.namespace),
      )
    } else {
      Manifest::read(&pointer.dir, &pointer.name).map_or_else(
        |why| format!("its manifest does not read: {why}"),
        |manifest| manifest.entries.iter().map(|entry| entry.full_name()).collect::<Vec<_>>().join(", "),
      )
    };
    let service = pointer
      .service
      .as_ref()
      .map_or_else(String::new, |service| format!(" for the service {service}"));

    said.push(format!(
      "{} {} from {}{service}: {entries}",
      pointer.name, pointer.version, pointer.origin
    ));
  }

  said.extend(faults.into_iter().map(|fault| format!("left out {fault}")));

  if pointers.is_empty() {
    said.push("No tool is registered.".to_owned());
  }

  Ok(())
}

// A version's name as an owner's machine names it: letters, digits, `-`, `_` and `.`, opening on no dot.
fn version_name(name: &str) -> bool {
  !name.is_empty()
    && !name.starts_with('.')
    && name.chars().all(|one| one.is_ascii_alphanumeric() || matches!(one, '-' | '_' | '.'))
}

// The owner's registry, refused where the account is no owner's name.
fn owned(registry: &Registry, owner: &str) -> Result<Registry, String> {
  if !owner_name(owner) {
    return Err(format!("{owner:?} is no account's name: letters, digits, `-` and `_`"));
  }

  Ok(registry.owned(owner))
}

/// An owner's personal tool's description copy put (docs/registry.md, 「Put a personal tool」): the manifest read as
/// a personal tool's, its entries held by no other of the owner's tools, the manifest and the files it names copied
/// into the version `version` names, or a fresh one, read again as the host reads it, then placed and pointed at
/// under the owner's registry's lock. A version of that name standing already is pointed at as it stands.
pub fn put_owned(
  registry: &Registry,
  owner: &str,
  folder: &Path,
  (version, app): (Option<&str>, Option<&str>),
  said: &mut Vec<String>,
) -> Result<(), String> {
  let owned = owned(registry, owner)?;

  if let Some(key) = app.filter(|key| !bmo_tool::manifest::app_key(key)) {
    return Err(format!("{key:?} is no app's key"));
  }

  let folder =
    fs::canonicalize(folder).map_err(|why| format!("{} could not be read: {why}", folder.display()))?;
  let name = manifest_name(&folder)?;
  let manifest =
    Manifest::read(&folder, &name).map_err(|why| format!("{name}: its manifest does not read: {why}"))?;

  if let Some(why) = schema_faults(&manifest).or_else(|| manifest.personal_faults().err()) {
    return Err(format!("{name}: {why}"));
  }

  let version = match version {
    Some(version) if version_name(version) => version.to_owned(),
    Some(version) => return Err(format!("{version:?} is no version's name")),
    None => fresh(),
  };

  clash(&owned, &name, &manifest)?;

  let placed = owned.version_dir(&name, &version);
  let staged = owned.versions().join(&name).join(format!(".staging-{version}-{}", fresh()));
  let staging = Scratch(Some(staged.clone()));

  if manifest.screen.is_some() {
    crate::screens::read_screen(&folder, &name).map_err(|why| format!("{name}: its screen: {why}"))?;
  }

  if !placed.join(MANIFEST).exists() {
    for file in manifest.description_files() {
      let to = staged.join(&file);

      if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
          .map_err(|why| format!("{} could not be made: {why}", parent.display()))?;
      }

      fs::copy(folder.join(&file), &to).map_err(|why| format!("{file} could not be copied: {why}"))?;
    }

    Manifest::read(&staged, &name)
      .map_err(|why| format!("{name}: its placed version does not read: {why}"))?;
  }

  let locked = owned.lock().map_err(|why| format!("the registry could not be locked: {why}"))?;
  // Read again under the lock: another put may have placed an entry of the owner's since.
  let standing = owned.get(&name);

  clash(&owned, &name, &manifest)?;

  if placed.join(MANIFEST).exists() {
    let kept = Manifest::read(&placed, &name)
      .map_err(|why| format!("{name}: its standing version {version} does not read: {why}"))?;

    if let Some(why) = schema_faults(&kept).or_else(|| kept.personal_faults().err()) {
      return Err(format!("{name}: its standing version {version}: {why}"));
    }
  } else {
    fs::rename(&staged, &placed)
      .map_err(|why| format!("{} could not be placed: {why}", placed.display()))?;
    staging.keep();
  }

  let pointer = Pointer {
    name: name.clone(),
    version: version.clone(),
    dir: placed.clone(),
    origin: format!("machine:{owner}"),
    service: None,
    app: app.map(str::to_owned),
    owner: Some(owner.to_owned()),
    put_at: now_ms(),
  };

  owned
    .point(&pointer)
    .map_err(|why| format!("the registry could not point at {name}: {why}"))?;
  drop(locked);

  said.push(match standing {
    Some(was) if was.version == version => format!("put {name} {version} again for {owner}"),
    Some(was) => format!("moved {owner}'s {name} from {} to {version}", was.version),
    None => format!("put {owner}'s {name} {version}"),
  });
  said.extend(manifest.entries.iter().map(|entry| format!("{name} serves {}", entry.full_name())));

  Ok(())
}

/// An owner's personal tool taken: its pointer removed under the owner's registry's lock, its versions left for the
/// host to take away once nothing holds them.
pub fn take_owned(
  registry: &Registry,
  owner: &str,
  name: &str,
  said: &mut Vec<String>,
) -> Result<(), String> {
  let owned = owned(registry, owner)?;
  let _locked = owned.lock().map_err(|why| format!("the registry could not be locked: {why}"))?;
  let Some(standing) = owned.get(name) else {
    return Err(format!("{owner} holds no personal tool {name}"));
  };

  owned
    .unpoint(name)
    .map_err(|why| format!("{name}'s pointer could not be taken away: {why}"))?;
  said.push(format!("took {owner}'s {name} {}", standing.version));

  Ok(())
}

/// One entry of a personal tool as an owner's listing shows it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ListedEntry {
  pub name: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
  pub description: String,
}

/// A personal tool as an owner's listing shows it: its pointer, its entries and its permissions.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listed {
  #[serde(flatten)]
  pub pointer: Pointer,
  pub entries: Vec<ListedEntry>,
  pub permissions: bmo_tool::manifest::PermissionsPart,
}

/// Every personal tool of an owner's, a line each, or as JSON where `json` asks.
pub fn list_owned(
  registry: &Registry,
  owner: &str,
  json: bool,
  said: &mut Vec<String>,
) -> Result<(), String> {
  let owned = owned(registry, owner)?;
  let (pointers, faults) = owned.entries();
  let listed: Vec<(Pointer, Result<Manifest, String>)> = pointers
    .into_iter()
    .map(|pointer| {
      let manifest = Manifest::read(&pointer.dir, &pointer.name).map_err(|why| why.to_string());

      (pointer, manifest)
    })
    .collect();

  if json {
    let rows: Vec<Listed> = listed
      .into_iter()
      .map(|(pointer, manifest)| {
        let (entries, permissions) = manifest.map_or_else(
          |_| (Vec::new(), bmo_tool::manifest::PermissionsPart::default()),
          |manifest| {
            let entries = manifest
              .entries
              .iter()
              .map(|entry| ListedEntry {
                name: entry.full_name(),
                title: entry.title.clone(),
                description: entry.description.clone(),
              })
              .collect();

            (entries, manifest.permissions)
          },
        );

        Listed {
          pointer,
          entries,
          permissions,
        }
      })
      .collect();

    said.push(serde_json::to_string_pretty(&rows).map_err(|why| why.to_string())?);
    return Ok(());
  }

  for (pointer, manifest) in &listed {
    let entries = manifest.as_ref().map_or_else(
      |why| format!("its manifest does not read: {why}"),
      |manifest| manifest.entries.iter().map(|entry| entry.full_name()).collect::<Vec<_>>().join(", "),
    );

    said.push(format!("{} {}: {entries}", pointer.name, pointer.version));
  }

  said.extend(faults.into_iter().map(|fault| format!("left out {fault}")));

  if listed.is_empty() {
    said.push(format!("{owner} holds no personal tool."));
  }

  Ok(())
}
