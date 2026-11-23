//! A company's catalogue of the personal apps its people share (docs/catalogue.md): one record an app, the source
//! bundle of each version shared, and the moment each reader last looked, under `tool-host/catalogue/<company>/`,
//! each change made under the catalogue's lock and each file written whole. The verbs `bmo-pocket catalogue` hands
//! its words to stand here; the host's call path reads none of it.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Component, Path, PathBuf};

use bmo_tool::manifest::{APP_KEY_MOST, AppPart, MANIFEST, Manifest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::chain::Chain;
use crate::placed::schema_faults;
use crate::records::now_ms;
use crate::registry::owner_name;

/// The folder under the host's own the companies' catalogues stand in, one each.
const CATALOGUE: &str = "catalogue";

/// The most bytes a source bundle's files hold together.
pub const BUNDLE_MOST: usize = 8 * 1024 * 1024;

/// A source bundle: every file of the app's folder by its path, as text (docs/catalogue.md, 「A source bundle」).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
  pub files: BTreeMap<String, String>,
}

/// What a version's manifest asks for beyond the app's own folders.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
  #[serde(default)]
  pub network: Vec<String>,
  #[serde(default)]
  pub secrets: Vec<String>,
}

/// The automatic review of a version: pending until written, written with its report, or failed with why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
  pub state: String,
  #[serde(default)]
  pub report: Option<String>,
  #[serde(default)]
  pub at: Option<u64>,
}

/// Whether a head must approve a version before anybody installs it, and who did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
  pub required: bool,
  #[serde(default)]
  pub by: Option<String>,
  #[serde(default)]
  pub at: Option<u64>,
}

/// One version shared.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
  pub version: String,
  pub source_hash: String,
  pub permissions: Permissions,
  pub shared_at: u64,
  pub review: Review,
  pub approval: Approval,
}

/// A receiver's copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Install {
  pub version: String,
  pub how: String,
  pub at: u64,
}

/// The request to make an app a platform app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Promotion {
  pub by: String,
  pub at: u64,
  #[serde(default)]
  pub note: String,
}

/// An app's record (docs/catalogue.md, 「An app's record」).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
  pub id: String,
  pub key: String,
  pub name: String,
  pub title: String,
  pub description: String,
  pub maker: String,
  pub owner: String,
  pub unit: String,
  pub targets: Value,
  pub versions: Vec<Version>,
  #[serde(default)]
  pub withdrawn: Option<u64>,
  #[serde(default)]
  pub installs: BTreeMap<String, Install>,
  #[serde(default)]
  pub promotion: Option<Promotion>,
  pub shared_at: u64,
  pub updated_at: u64,
}

impl Record {
  fn version(&self, version: Option<&str>) -> Result<&Version, String> {
    match version {
      Some(name) => self
        .versions
        .iter()
        .find(|one| one.version == name)
        .ok_or_else(|| format!("the app {} has no version {name}", self.id)),
      None => self.versions.last().ok_or_else(|| format!("the app {} has no version", self.id)),
    }
  }

  fn version_mut(&mut self, version: &str) -> Result<&mut Version, String> {
    let id = self.id.clone();

    self
      .versions
      .iter_mut()
      .find(|one| one.version == version)
      .ok_or_else(|| format!("the app {id} has no version {version}"))
  }
}

/// The hash a share records of a bundle (docs/catalogue.md, 「The source hash」): the SHA-256 of its files in the byte
/// order of their paths, each its path, a zero byte, its length in decimal, a zero byte and its bytes.
pub fn source_hash(files: &BTreeMap<String, String>) -> String {
  let mut hasher = Sha256::new();

  for (path, text) in files {
    hasher.update(path.as_bytes());
    hasher.update([0]);
    hasher.update(text.len().to_string().as_bytes());
    hasher.update([0]);
    hasher.update(text.as_bytes());
  }

  hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The key a shared app stands under on the delegation chain (docs/catalogue.md, 「The app on the delegation
/// chain」): `shared-<company>-<maker>-<name>`, the name's `_` written `-`, or its hash's first twelve hex digits where
/// that would be longer than an app key holds.
pub fn app_key(company: &str, maker: &str, name: &str) -> String {
  let lower = |text: &str| text.to_ascii_lowercase().replace('_', "-");
  let whole = format!("shared-{}-{}-{}", lower(company), lower(maker), lower(name));

  if whole.len() <= APP_KEY_MOST {
    return whole;
  }

  let digest: String = Sha256::digest(name.as_bytes())
    .iter()
    .take(6)
    .map(|byte| format!("{byte:02x}"))
    .collect();

  format!("shared-{}-{}-{digest}", lower(company), lower(maker))
}

// A path inside a folder, made of folder and file names alone, so it never climbs out.
fn inside(path: &str) -> bool {
  !path.is_empty() && Path::new(path).components().all(|part| matches!(part, Component::Normal(_)))
}

// An app's name: a lower-case letter, then lower-case letters, digits and `_`.
fn app_name(name: &str) -> bool {
  name.chars().next().is_some_and(|first| first.is_ascii_lowercase())
    && name.chars().all(|one| one.is_ascii_lowercase() || one.is_ascii_digit() || one == '_')
}

// A version's name: letters, digits, `-`, `_` and `.`, opening on no dot.
fn version_name(name: &str) -> bool {
  !name.is_empty()
    && !name.starts_with('.')
    && name.chars().all(|one| one.is_ascii_alphanumeric() || matches!(one, '-' | '_' | '.'))
}

/// The catalogue's lock, held until it is dropped.
pub struct Locked(File);

impl Drop for Locked {
  fn drop(&mut self) {
    // SAFETY: an unlock of the descriptor this value owns; an error is no harm, as closing it unlocks too.
    unsafe {
      libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
    }
  }
}

/// One company's catalogue.
#[derive(Clone, Debug)]
pub struct Catalogue {
  pub root: PathBuf,
  pub company: String,
}

// A file written whole: beside its place under a name of its own, then renamed over the old.
fn write_whole(path: &Path, text: &str) -> io::Result<()> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)?;
  }

  let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
  let staged = path.with_file_name(format!(".{name}.next"));

  fs::write(&staged, text)?;
  fs::rename(&staged, path)
}

/// The folder every company's catalogue stands in under the data root.
pub fn catalogues(data_root: &Path) -> PathBuf {
  data_root.join("tool-host").join(CATALOGUE)
}

impl Catalogue {
  /// The company's catalogue under the data root; a company's name is a unit's, letters, digits, `-` and `_`.
  pub fn of(data_root: &Path, company: &str) -> Result<Self, String> {
    if !owner_name(company) {
      return Err(format!("{company:?} is no company's name"));
    }

    Ok(Self {
      root: catalogues(data_root).join(company),
      company: company.to_owned(),
    })
  }

  fn record_file(&self, id: &str) -> PathBuf {
    self.root.join("apps").join(format!("{id}.json"))
  }

  fn source_file(&self, id: &str, version: &str) -> PathBuf {
    self.root.join("sources").join(id).join(format!("{version}.json"))
  }

  fn seen_file(&self, account: &str) -> PathBuf {
    self.root.join("seen").join(format!("{account}.json"))
  }

  /// The catalogue's lock, waited for: every change to it is made under it.
  pub fn lock(&self) -> io::Result<Locked> {
    fs::create_dir_all(&self.root)?;

    let file = OpenOptions::new()
      .create(true)
      .truncate(false)
      .write(true)
      .open(self.root.join("catalogue.lock"))?;

    // SAFETY: a lock on the descriptor `file` owns, which it keeps until `Locked` is dropped.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
      return Err(io::Error::last_os_error());
    }

    Ok(Locked(file))
  }

  /// The app's record, where one reads.
  pub fn get(&self, id: &str) -> Option<Record> {
    if !owner_name(id) {
      return None;
    }

    serde_json::from_str(&fs::read_to_string(self.record_file(id)).ok()?).ok()
  }

  fn must(&self, id: &str) -> Result<Record, String> {
    self
      .get(id)
      .ok_or_else(|| format!("the catalogue of {} holds no app {id}", self.company))
  }

  fn put(&self, record: &mut Record) -> Result<(), String> {
    record.updated_at = now_ms();

    let text = serde_json::to_string_pretty(record).map_err(|why| why.to_string())?;

    write_whole(&self.record_file(&record.id), &text)
      .map_err(|why| format!("the app {} could not be written: {why}", record.id))
  }

  /// Every app's record, by id.
  pub fn list(&self) -> Vec<Record> {
    let mut files: Vec<PathBuf> = fs::read_dir(self.root.join("apps"))
      .into_iter()
      .flatten()
      .flatten()
      .map(|entry| entry.path())
      .filter(|path| {
        path.extension().is_some_and(|extension| extension == "json")
          && !path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.'))
      })
      .collect();

    files.sort();
    files
      .iter()
      .filter_map(|file| serde_json::from_str(&fs::read_to_string(file).ok()?).ok())
      .collect()
  }

  // One change to an app's record, made under the lock and written whole.
  fn change(
    &self,
    id: &str,
    change: impl FnOnce(&mut Record) -> Result<(), String>,
  ) -> Result<Record, String> {
    let _locked = self.lock().map_err(|why| format!("the catalogue could not be locked: {why}"))?;
    let mut record = self.must(id)?;

    change(&mut record)?;
    self.put(&mut record)?;
    Ok(record)
  }

  /// The moment the reader last looked at what was shared with them, where they did.
  pub fn seen_at(&self, account: &str) -> Option<u64> {
    let text = fs::read_to_string(self.seen_file(account)).ok()?;

    serde_json::from_str::<Value>(&text).ok()?["at"].as_u64()
  }

  /// The reader's looking kept as now.
  pub fn seen(&self, account: &str) -> Result<u64, String> {
    if !owner_name(account) {
      return Err(format!("{account:?} is no account's name"));
    }

    let at = now_ms();

    write_whole(&self.seen_file(account), &json!({ "at": at }).to_string())
      .map_err(|why| format!("the reader's looking could not be kept: {why}"))?;
    Ok(at)
  }

  /// A version's bundle as kept, its hash reckoned again and refused where it is not the one recorded.
  pub fn source(&self, id: &str, version: Option<&str>) -> Result<Value, String> {
    let record = self.must(id)?;
    let shared = record.version(version)?;
    let file = self.source_file(id, &shared.version);
    let bundle: Bundle = fs::read_to_string(&file)
      .map_err(|why| why.to_string())
      .and_then(|text| serde_json::from_str(&text).map_err(|why| why.to_string()))
      .map_err(|why| format!("the source of {id} {} does not read: {why}", shared.version))?;
    let hash = source_hash(&bundle.files);

    if hash != shared.source_hash {
      return Err(format!(
        "the source of {id} {} kept here does not match the hash recorded as it was shared",
        shared.version
      ));
    }

    Ok(json!({ "version": shared.version, "sourceHash": hash, "files": bundle.files }))
  }
}

/// One share as it is asked.
#[derive(Clone, Debug)]
pub struct Share<'a> {
  pub maker: &'a str,
  pub unit: &'a str,
  pub version: &'a str,
  pub targets: Value,
  pub bundle: Bundle,
  pub review_required: bool,
}

// The bundle's files written into a scratch folder, its manifest read there as a personal tool's, and the folder
// taken away again.
fn read_manifest(scratch: &Path, bundle: &Bundle) -> Result<Manifest, String> {
  let _ = fs::remove_dir_all(scratch);

  let written = (|| {
    for (path, text) in &bundle.files {
      let to = scratch.join(path);

      if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
      }

      fs::write(to, text)?;
    }

    io::Result::Ok(())
  })();
  let read = written.map_err(|why| format!("the bundle could not be read: {why}")).and_then(|()| {
    let text = fs::read_to_string(scratch.join(MANIFEST))
      .map_err(|why| format!("the bundle holds no {MANIFEST}: {why}"))?;
    let named: toml::Table = text.parse().map_err(|why| format!("its {MANIFEST} does not read: {why}"))?;
    let name = named
      .get("tool")
      .and_then(|tool| tool.get("name"))
      .and_then(toml::Value::as_str)
      .ok_or_else(|| format!("its {MANIFEST} names no tool"))?
      .to_owned();

    Manifest::read(scratch, &name).map_err(|why| format!("its manifest does not read: {why}"))
  });

  let _ = fs::remove_dir_all(scratch);
  read
}

/// A version of a maker's app shared into the catalogue: the bundle checked, the app's two settings declared on the
/// chain, neither offered nor on by default, the bundle kept and the version recorded with its hash. Answers the record as it stands.
pub fn share(
  catalogue: &Catalogue,
  chain: Option<&Chain>,
  asked: &Share,
  said: &mut Vec<String>,
) -> Result<Record, String> {
  if !owner_name(asked.maker) || !owner_name(asked.unit) {
    return Err("the maker and the unit are named by letters, digits, `-` and `_`".to_owned());
  }

  if !version_name(asked.version) {
    return Err(format!("{:?} is no version's name", asked.version));
  }

  if let Some(path) = asked.bundle.files.keys().find(|path| !inside(path)) {
    return Err(format!("the bundle holds a file {path:?} outside the app's folder"));
  }

  if asked.bundle.files.values().map(String::len).sum::<usize>() > BUNDLE_MOST {
    return Err(format!("the bundle holds more than {BUNDLE_MOST} bytes"));
  }

  let scratch = catalogue.root.join(".reading").join(format!("{}-{}", now_ms(), std::process::id()));
  let manifest = read_manifest(&scratch, &asked.bundle)?;
  let name = manifest.tool.name.clone();

  if !app_name(&name) {
    return Err(format!("{name:?} is no app's name"));
  }

  if let Some(why) = schema_faults(&manifest).or_else(|| manifest.personal_faults().err()) {
    return Err(format!("{name}: {why}"));
  }

  let id = format!("{}-{name}", asked.maker);
  let key = app_key(&catalogue.company, asked.maker, &name);
  let first = manifest.entries.first();
  let (title, description) = match &manifest.app {
    Some(app) => (app.name.clone(), app.description.clone()),
    None => (
      first.and_then(|entry| entry.title.clone()).unwrap_or_else(|| name.clone()),
      first.map(|entry| entry.description.clone()).unwrap_or_default(),
    ),
  };
  let Some(chain) = chain else {
    return Err(format!(
      "the settings of the app {key} could not be declared: no account service is named"
    ));
  };
  let app = AppPart {
    key: key.clone(),
    name: title.clone(),
    description: if description.trim().is_empty() { title.clone() } else { description.clone() },
    icon: None,
    offered: false,
    on: false,
  };

  said.extend(chain.declare(&app).map_err(|why| format!("the app {key}: {why}"))?);

  let _locked = catalogue.lock().map_err(|why| format!("the catalogue could not be locked: {why}"))?;
  let now = now_ms();
  let mut record = catalogue.get(&id).unwrap_or_else(|| Record {
    id: id.clone(),
    key: key.clone(),
    name: name.clone(),
    title: title.clone(),
    description: description.clone(),
    maker: asked.maker.to_owned(),
    owner: asked.maker.to_owned(),
    unit: asked.unit.to_owned(),
    targets: Value::Array(Vec::new()),
    versions: Vec::new(),
    withdrawn: None,
    installs: BTreeMap::new(),
    promotion: None,
    shared_at: now,
    updated_at: now,
  });

  if record.versions.iter().any(|one| one.version == asked.version) {
    return Err(format!("the app {id} has shared version {} already", asked.version));
  }

  let hash = source_hash(&asked.bundle.files);
  let bundle = serde_json::to_string(&asked.bundle).map_err(|why| why.to_string())?;

  write_whole(&catalogue.source_file(&id, asked.version), &bundle)
    .map_err(|why| format!("the source of {id} could not be kept: {why}"))?;

  record.title = title;
  record.description = description;
  record.unit = asked.unit.to_owned();
  record.targets = asked.targets.clone();
  record.withdrawn = None;
  record.versions.push(Version {
    version: asked.version.to_owned(),
    source_hash: hash.clone(),
    permissions: Permissions {
      network: manifest.permissions.network.clone(),
      secrets: manifest.permissions.secrets.clone(),
    },
    shared_at: now,
    review: Review {
      state: "pending".to_owned(),
      report: None,
      at: None,
    },
    approval: Approval {
      required: asked.review_required,
      by: None,
      at: None,
    },
  });
  catalogue.put(&mut record)?;
  said.push(format!("shared {id} version {} as {hash}", asked.version));
  Ok(record)
}

/// The app withdrawn: no new install from now; every copy goes on.
pub fn withdraw(catalogue: &Catalogue, id: &str) -> Result<Record, String> {
  catalogue.change(id, |record| {
    record.withdrawn.get_or_insert(now_ms());
    Ok(())
  })
}

/// A version's review written: `written` with its report, or `failed` with why none was.
pub fn review(
  catalogue: &Catalogue,
  id: &str,
  version: &str,
  state: &str,
  report: &str,
) -> Result<Record, String> {
  if !matches!(state, "written" | "failed") {
    return Err(format!("{state:?} is no review's state: written or failed"));
  }

  catalogue.change(id, |record| {
    let shared = record.version_mut(version)?;

    shared.review = Review {
      state: state.to_owned(),
      report: Some(report.to_owned()),
      at: Some(now_ms()),
    };
    Ok(())
  })
}

/// A version approved by a head.
pub fn approve(catalogue: &Catalogue, id: &str, version: &str, by: &str) -> Result<Record, String> {
  catalogue.change(id, |record| {
    let shared = record.version_mut(version)?;

    shared.approval.by = Some(by.to_owned());
    shared.approval.at = Some(now_ms());
    Ok(())
  })
}

/// A receiver's copy kept: the version it stands on and how it came.
pub fn install(
  catalogue: &Catalogue,
  id: &str,
  account: &str,
  version: &str,
  how: &str,
) -> Result<Record, String> {
  if !owner_name(account) {
    return Err(format!("{account:?} is no account's name"));
  }

  if !matches!(how, "self" | "unit") {
    return Err(format!("{how:?} is no way a copy comes: self or unit"));
  }

  catalogue.change(id, |record| {
    record.version(Some(version))?;
    record.installs.insert(
      account.to_owned(),
      Install {
        version: version.to_owned(),
        how: how.to_owned(),
        at: now_ms(),
      },
    );
    Ok(())
  })
}

/// A receiver's copy let go.
pub fn uninstall(catalogue: &Catalogue, id: &str, account: &str) -> Result<Record, String> {
  catalogue.change(id, |record| {
    record.installs.remove(account);
    Ok(())
  })
}

/// Every app the leaver owns passed to the account taking them over; answers the ids passed.
pub fn pass(catalogue: &Catalogue, from: &str, to: &str) -> Result<Vec<String>, String> {
  if !owner_name(to) {
    return Err(format!("{to:?} is no account's name"));
  }

  let _locked = catalogue.lock().map_err(|why| format!("the catalogue could not be locked: {why}"))?;
  let mut passed = Vec::new();

  for mut record in catalogue.list() {
    if record.owner == from {
      record.owner = to.to_owned();
      catalogue.put(&mut record)?;
      passed.push(record.id);
    }
  }

  Ok(passed)
}

/// The request to make an app a platform app kept, the latest standing.
pub fn promote(catalogue: &Catalogue, id: &str, by: &str, note: &str) -> Result<Record, String> {
  catalogue.change(id, |record| {
    record.promotion = Some(Promotion {
      by: by.to_owned(),
      at: now_ms(),
      note: note.to_owned(),
    });
    Ok(())
  })
}

/// Every company's requests to make an app a platform app, newest first.
pub fn promotions(data_root: &Path) -> Vec<Value> {
  let mut found: Vec<Value> = fs::read_dir(catalogues(data_root))
    .into_iter()
    .flatten()
    .flatten()
    .filter_map(|entry| entry.file_name().into_string().ok())
    .filter(|company| owner_name(company))
    .filter_map(|company| Catalogue::of(data_root, &company).ok())
    .flat_map(|catalogue| {
      catalogue
        .list()
        .into_iter()
        .filter_map(|record| {
          let promotion = record.promotion?;

          Some(json!({
            "company": catalogue.company,
            "id": record.id,
            "title": record.title,
            "maker": record.maker,
            "by": promotion.by,
            "at": promotion.at,
            "note": promotion.note,
          }))
        })
        .collect::<Vec<_>>()
    })
    .collect();

  found.sort_by_key(|one| std::cmp::Reverse(one["at"].as_u64().unwrap_or(0)));
  found
}
