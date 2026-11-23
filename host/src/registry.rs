//! The registry (docs/registry.md): one pointer per registered tool under the data root's `tool-host/registry/`,
//! each naming the placed version of the tool the host serves, written whole under the registry's lock.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The pointer files' extension.
const POINTER: &str = "json";

/// The folder under the host's own the owners' registries of personal tools stand in, one each.
const OWNERS: &str = "owners";

/// An owner's name as a folder holds it: letters, digits, `-` and `_`, as an account's key is written.
pub fn owner_name(name: &str) -> bool {
  !name.is_empty() && name.chars().all(|one| one.is_ascii_alphanumeric() || one == '-' || one == '_')
}

/// Where a tool's registration points, its keys in camel case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pointer {
  /// The tool's name, its manifest's.
  pub name: String,
  /// The placed version it points at.
  pub version: String,
  /// That version's folder.
  pub dir: PathBuf,
  /// Where it was put from: `path:<folder>`, `git:<address>` or `name:<name>`.
  pub origin: String,
  /// The repository whose own tool it is, where a service registered it.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub service: Option<String>,
  /// The account whose personal tool it is, where it is one: the folder of the owner's registry it stands in names
  /// the same account.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub owner: Option<String>,
  /// The key of the shared app a received personal tool belongs to, in place of its manifest's own `[app]`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub app: Option<String>,
  /// When it was put, in epoch milliseconds; a pointer naming it `put_at` reads too.
  #[serde(alias = "put_at")]
  pub put_at: u64,
}

/// The registry under one data root.
#[derive(Clone, Debug)]
pub struct Registry {
  /// The host's own folder under the data root, `tool-host/`.
  pub root: PathBuf,
}

/// The registry's lock, held until it is dropped.
pub struct Locked(File);

impl Drop for Locked {
  fn drop(&mut self) {
    // SAFETY: an unlock of the descriptor this value owns; an error is no harm, as closing it unlocks too.
    unsafe {
      libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
    }
  }
}

impl Registry {
  /// The registry of the data root.
  pub fn of(data_root: &Path) -> Self {
    Self {
      root: data_root.join("tool-host"),
    }
  }

  /// The folder every owner's registry of personal tools stands in.
  pub fn owners_root(&self) -> PathBuf {
    self.root.join(OWNERS)
  }

  /// The registry of one owner's personal tools (docs/registry.md, 「Personal tools」): the same layout, under the
  /// owner's folder.
  pub fn owned(&self, owner: &str) -> Self {
    Self {
      root: self.owners_root().join(owner),
    }
  }

  /// Every owner's registry standing, by the owner's name.
  pub fn owners(&self) -> Vec<(String, Self)> {
    let mut found: Vec<String> = fs::read_dir(self.owners_root())
      .into_iter()
      .flatten()
      .flatten()
      .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
      .filter_map(|entry| entry.file_name().into_string().ok())
      .filter(|name| owner_name(name))
      .collect();

    found.sort();
    found.into_iter().map(|owner| (owner.clone(), self.owned(&owner))).collect()
  }

  /// Every pointer of the registry and of every owner's registry beside it, each that reads.
  pub fn every_pointer(&self) -> Vec<Pointer> {
    let mut every = self.entries().0;

    for (_, owned) in self.owners() {
      every.extend(owned.entries().0);
    }

    every
  }

  /// Whether a folder is a placed version of this registry or of an owner's beside it: the only folders the host
  /// ever takes away.
  pub fn holds_version(&self, dir: &Path) -> bool {
    dir.starts_with(self.versions())
      || dir
        .strip_prefix(self.owners_root())
        .is_ok_and(|under| under.components().nth(1).is_some_and(|part| part.as_os_str() == "tools"))
  }

  /// The folder of the pointers.
  pub fn pointers(&self) -> PathBuf {
    self.root.join("registry")
  }

  /// The folder each tool's placed versions stand in.
  pub fn versions(&self) -> PathBuf {
    self.root.join("tools")
  }

  /// Where a put clones and builds.
  pub fn build(&self) -> PathBuf {
    self.root.join("build")
  }

  /// The folder a placed version of a tool stands in.
  pub fn version_dir(&self, name: &str, version: &str) -> PathBuf {
    self.versions().join(name).join(version)
  }

  fn pointer_file(&self, name: &str) -> PathBuf {
    self.pointers().join(format!("{name}.{POINTER}"))
  }

  /// The registry's lock, waited for: every change to the registry, and the host's taking away of the versions no
  /// pointer names, is made under it.
  pub fn lock(&self) -> io::Result<Locked> {
    fs::create_dir_all(&self.root)?;

    let file = OpenOptions::new()
      .create(true)
      .truncate(false)
      .write(true)
      .open(self.root.join("registry.lock"))?;

    // SAFETY: a lock on the descriptor `file` owns, which it keeps until `Locked` is dropped.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
      return Err(io::Error::last_os_error());
    }

    Ok(Locked(file))
  }

  /// Every pointer that reads, by name, and why each that does not read was left out.
  pub fn entries(&self) -> (Vec<Pointer>, Vec<String>) {
    let mut found = Vec::new();
    let mut faults = Vec::new();
    let mut files: Vec<PathBuf> = fs::read_dir(self.pointers())
      .into_iter()
      .flatten()
      .flatten()
      .map(|entry| entry.path())
      .filter(|path| {
        path.extension().is_some_and(|extension| extension == POINTER)
          && !path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.'))
      })
      .collect();

    files.sort();

    for file in files {
      let named = file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
      let read = fs::read_to_string(&file)
        .map_err(|why| why.to_string())
        .and_then(|text| serde_json::from_str::<Pointer>(&text).map_err(|why| why.to_string()));

      match read {
        Ok(pointer) if pointer.name == named => found.push(pointer),
        Ok(pointer) => faults.push(format!("registry/{named}: it points for `{}`", pointer.name)),
        Err(why) => faults.push(format!("registry/{named}: {why}")),
      }
    }

    (found, faults)
  }

  /// The pointer of the tool of this name, where one reads.
  pub fn get(&self, name: &str) -> Option<Pointer> {
    let text = fs::read_to_string(self.pointer_file(name)).ok()?;

    serde_json::from_str(&text).ok()
  }

  /// The pointer written whole in one step: beside its place under a name of its own, then renamed over the old.
  pub fn point(&self, pointer: &Pointer) -> io::Result<()> {
    fs::create_dir_all(self.pointers())?;

    let staged = self.pointers().join(format!(".{}.{POINTER}.next", pointer.name));
    let text = serde_json::to_string_pretty(pointer).map_err(io::Error::other)?;

    fs::write(&staged, text)?;
    fs::rename(&staged, self.pointer_file(&pointer.name))
  }

  /// The pointer of the tool of this name taken away; whether there was one.
  pub fn unpoint(&self, name: &str) -> io::Result<bool> {
    match fs::remove_file(self.pointer_file(name)) {
      Ok(()) => Ok(true),
      Err(why) if why.kind() == io::ErrorKind::NotFound => Ok(false),
      Err(why) => Err(why),
    }
  }
}
