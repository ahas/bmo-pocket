//! The registry (docs/registry.md): one pointer per registered tool under the data root's `tool-host/registry/`,
//! each naming the placed version of the tool the host serves, written whole under the registry's lock.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The pointer files' extension.
const POINTER: &str = "json";

/// Where a tool's registration points.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
  /// When it was put, in epoch milliseconds.
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
