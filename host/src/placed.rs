//! The placed tools' catalogue (docs/mcp-host.md, 「The catalogue」): read from the tools root — each folder's
//! `.placed/` alone — and from the registry's pointers (docs/registry.md), and read again whole once the changes
//! under either settle, so a tool placed, replaced or taken away is served from the next call with nothing
//! restarted. A registered version that has left the catalogue is taken away once nothing holds it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

pub use bmo_tool::manifest::PLACED;
use bmo_tool::manifest::{MANIFEST, Manifest};
use notify::event::ModifyKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::registry::Registry;

/// A tool as it is placed: its folder's name, where it is placed, and its manifest.
#[derive(Debug)]
pub struct PlacedTool {
  pub folder: String,
  pub dir: PathBuf,
  pub manifest: Manifest,
}

/// Every placed tool and shared helper, as one reading of the tools root and the registry found them.
#[derive(Debug, Default)]
pub struct Catalogue {
  pub tools: Vec<Arc<PlacedTool>>,
  pub helpers: BTreeMap<String, Arc<PlacedTool>>,
}

// The folders standing in `dir`, by name.
fn folders(dir: &Path) -> Vec<(String, PathBuf)> {
  let mut found: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
    .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
    .collect();

  found.sort();
  found
}

/// What is wrong with a manifest's schemas as JSON Schema, where anything is.
pub fn schema_faults(manifest: &Manifest) -> Option<String> {
  manifest.entries.iter().find_map(|entry| {
    std::iter::once(&entry.input_schema)
      .chain(&entry.output_schema)
      .find_map(|schema| jsonschema::validator_for(schema).err())
      .map(|why| format!("{}: its schema is not a JSON Schema: {why}", entry.full_name()))
  })
}

impl Catalogue {
  /// The catalogue the tools root holds, and why each tool left out was left out.
  pub fn read(dir: &Path) -> (Self, Vec<String>) {
    Self::read_all(Some(dir), None)
  }

  /// The catalogue the tools root and the registry hold together, the tools root's read first, and why each tool
  /// left out was left out.
  pub fn read_all(dir: Option<&Path>, registry: Option<&Registry>) -> (Self, Vec<String>) {
    let mut catalogue = Self::default();
    let mut faults = Vec::new();
    let mut held = BTreeSet::new();

    for (folder, path) in dir.map(folders).unwrap_or_default() {
      let placed = path.join(PLACED);

      if folder.starts_with('_') || !placed.join(MANIFEST).exists() {
        continue;
      }

      match Manifest::read(&placed, &folder) {
        Ok(manifest) if manifest.helper.is_some() => {
          faults.push(format!("tools/{folder}: a shared helper is found through the registry alone"));
        }
        Ok(manifest) => {
          catalogue.take_in(format!("tools/{folder}"), folder, placed, manifest, &mut held, &mut faults)
        }
        Err(why) => faults.push(format!("tools/{folder}: {why}")),
      }
    }

    let (pointers, unread) = registry.map(Registry::entries).unwrap_or_default();

    faults.extend(unread);

    for pointer in pointers {
      let at = format!("registry/{}", pointer.name);

      if !pointer.dir.join(MANIFEST).exists() {
        faults.push(format!("{at}: its placed folder {} is gone", pointer.dir.display()));
        continue;
      }

      match Manifest::read(&pointer.dir, &pointer.name) {
        Ok(manifest) if manifest.helper.is_some() => {
          catalogue.helpers.insert(
            pointer.name.clone(),
            Arc::new(PlacedTool {
              folder: pointer.name,
              dir: pointer.dir,
              manifest,
            }),
          );
        }
        Ok(manifest) => catalogue.take_in(at, pointer.name, pointer.dir, manifest, &mut held, &mut faults),
        Err(why) => faults.push(format!("{at}: {why}")),
      }
    }

    (catalogue, faults)
  }

  // One tool taken into the catalogue where its schemas read and no entry of it is another tool's already.
  fn take_in(
    &mut self,
    at: String,
    folder: String,
    dir: PathBuf,
    manifest: Manifest,
    held: &mut BTreeSet<String>,
    faults: &mut Vec<String>,
  ) {
    if let Some(why) = schema_faults(&manifest) {
      faults.push(format!("{at}: {why}"));
      return;
    }

    let names: Vec<String> = manifest.entries.iter().map(|entry| entry.full_name()).collect();

    if let Some(taken) = names.iter().find(|name| held.contains(*name)) {
      faults.push(format!("{at}: {taken} is another tool's already"));
      return;
    }

    held.extend(names);
    self.tools.push(Arc::new(PlacedTool {
      folder,
      dir,
      manifest,
    }));
  }

  /// Every namespace a placed entry is declared in.
  pub fn namespaces(&self) -> BTreeSet<String> {
    self
      .tools
      .iter()
      .flat_map(|tool| tool.manifest.entries.iter().map(|entry| entry.namespace.clone()))
      .chain(
        self
          .tools
          .iter()
          .filter_map(|tool| tool.manifest.resources.as_ref().map(|res| res.namespace.clone())),
      )
      .collect()
  }

  // Every placed folder it serves from, tools and helpers alike.
  fn dirs(&self) -> BTreeSet<PathBuf> {
    self.tools.iter().chain(self.helpers.values()).map(|tool| tool.dir.clone()).collect()
  }
}

/// Whether a change under a watched folder can move what is placed: a file or folder made, written, renamed or
/// taken away. Opening and reading one — the host reading a manifest, a tool run reading its own files — moves
/// nothing, and is never a reason to read again.
pub fn moves(kind: &EventKind) -> bool {
  !matches!(kind, EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(_)))
}

/// The catalogue the host serves from, and where it is read.
pub struct Placed {
  /// The tools root; none where the host serves no tool from one.
  pub dir: Option<PathBuf>,
  /// The registry read beside it; none where the host serves no registered tool.
  pub registry: Option<Registry>,
  /// Where each call's call-back socket and each helper's socket are bound.
  pub sockets: PathBuf,
  current: RwLock<Arc<Catalogue>>,
  /// The registered versions that have left the catalogue, each taken away once nothing else holds it.
  retired: Mutex<Vec<Arc<PlacedTool>>>,
}

impl Placed {
  pub fn new(dir: Option<PathBuf>, sockets: PathBuf) -> Self {
    Self {
      dir,
      registry: None,
      sockets,
      current: RwLock::new(Arc::new(Catalogue::default())),
      retired: Mutex::new(Vec::new()),
    }
  }

  /// The same, reading the registry beside the tools root.
  pub fn registered(mut self, registry: Registry) -> Self {
    self.registry = Some(registry);
    self
  }

  /// The catalogue as the last reading left it.
  pub fn current(&self) -> Arc<Catalogue> {
    self.current.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
  }

  /// The tools root and the registry read again whole and put in place at once, each tool left out said in the
  /// log; every registered version that left the catalogue is retired, and taken away once nothing holds it.
  pub fn reread(&self) {
    if self.dir.is_none() && self.registry.is_none() {
      return;
    }

    let (catalogue, faults) = Catalogue::read_all(self.dir.as_deref(), self.registry.as_ref());

    for fault in &faults {
      eprintln!("mcp-host: left out {fault}");
    }

    let read = match (&self.dir, &self.registry) {
      (Some(dir), Some(_)) => format!("{} and the registry", dir.display()),
      (Some(dir), None) => dir.display().to_string(),
      (None, _) => "the registry".to_owned(),
    };

    eprintln!(
      "mcp-host: read {read}: {} tools and {} shared helpers placed",
      catalogue.tools.len(),
      catalogue.helpers.len()
    );

    let kept = catalogue.dirs();
    let old = std::mem::replace(
      &mut *self.current.write().unwrap_or_else(std::sync::PoisonError::into_inner),
      Arc::new(catalogue),
    );
    let leaving: Vec<Arc<PlacedTool>> = old
      .tools
      .iter()
      .chain(old.helpers.values())
      .filter(|tool| !kept.contains(&tool.dir) && self.is_version(&tool.dir))
      .cloned()
      .collect();

    drop(old);
    self.retired.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(leaving);
    self.sweep();
  }

  // Whether a folder is one of the registry's placed versions, the only folders the host ever takes away.
  fn is_version(&self, dir: &Path) -> bool {
    self.registry.as_ref().is_some_and(|registry| dir.starts_with(registry.versions()))
  }

  /// Every retired version nothing holds any longer taken away: no call, server or helper started from it runs.
  pub fn sweep(&self) {
    let serving = self.current().dirs();
    let mut retired = self.retired.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    retired.retain(|tool| {
      // A folder the catalogue serves again is no longer retired, whatever held it.
      if serving.contains(&tool.dir) {
        return false;
      }

      if Arc::strong_count(tool) > 1 {
        return true;
      }

      match std::fs::remove_dir_all(&tool.dir) {
        Ok(()) => eprintln!("mcp-host: took away the retired {}", tool.dir.display()),
        Err(why) => {
          eprintln!("mcp-host: the retired {} could not be taken away: {why}", tool.dir.display())
        }
      }

      false
    });
  }

  /// Every registered version no pointer names taken away, under the registry's lock: at the host's start, when no
  /// call can hold one.
  pub fn clear_unnamed(&self) {
    let Some(registry) = &self.registry else {
      return;
    };
    let Ok(_locked) = registry.lock() else {
      return;
    };
    let (pointers, _) = registry.entries();
    let named: BTreeSet<PathBuf> = pointers.into_iter().map(|pointer| pointer.dir).collect();

    for (_, tool) in folders(&registry.versions()) {
      for (version, dir) in folders(&tool) {
        if version.starts_with('.') || named.contains(&dir) {
          continue;
        }

        match std::fs::remove_dir_all(&dir) {
          Ok(()) => eprintln!("mcp-host: took away {}, which no pointer names", dir.display()),
          Err(why) => eprintln!("mcp-host: {} could not be taken away: {why}", dir.display()),
        }
      }
    }
  }

  /// The tools root and the registry read now and again once every run of changes under either has settled for
  /// `settle`, for as long as the host runs. A tools root taken away whole is made again and watched anew, and one
  /// whose real path moves — a link on its path flipped onto another release — is watched where it now lies and
  /// read there.
  pub fn watch(self: &Arc<Self>, settle: Duration) -> notify::Result<()> {
    if self.dir.is_none() && self.registry.is_none() {
      return Ok(());
    }

    if let Some(dir) = &self.dir {
      std::fs::create_dir_all(dir).map_err(notify::Error::io)?;
    }

    let links = self.dir.as_deref().map(links_on).unwrap_or_default();
    let folders: Vec<PathBuf> =
      links.iter().filter_map(|link| link.parent().map(Path::to_path_buf)).collect();
    let mut real = match &self.dir {
      Some(dir) => Some(std::fs::canonicalize(dir).map_err(notify::Error::io)?),
      None => None,
    };
    let (changed, mut changes) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut watcher: RecommendedWatcher = {
      let (links, folders) = (links.clone(), folders.clone());

      notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| moves(&event.kind) && concerns(&event.paths, &links, &folders)) {
          let _ = changed.send(());
        }
      })?
    };

    if let Some(real) = &real {
      watcher.watch(real, RecursiveMode::Recursive)?;
    }

    for folder in &folders {
      watcher.watch(folder, RecursiveMode::NonRecursive)?;
    }

    if let Some(registry) = &self.registry {
      std::fs::create_dir_all(registry.pointers()).map_err(notify::Error::io)?;
      watcher.watch(&registry.pointers(), RecursiveMode::NonRecursive)?;
    }

    self.reread();
    self.clear_unnamed();

    let placed = self.clone();
    let dir = self.dir.clone();

    tokio::spawn(async move {
      while changes.recv().await.is_some() {
        loop {
          match tokio::time::timeout(settle, changes.recv()).await {
            Ok(Some(())) => {}
            Ok(None) => return,
            Err(_) => break,
          }
        }

        if let (Some(dir), Some(was)) = (&dir, &mut real) {
          // A tools root taken away whole is made again, and its watch went with it.
          let remade = !dir.is_dir();

          if remade && let Err(why) = std::fs::create_dir_all(dir) {
            eprintln!("mcp-host: {} could not be made again: {why}", dir.display());
          }

          match std::fs::canonicalize(dir) {
            Ok(now) if remade || now != *was => {
              let _ = watcher.unwatch(was);

              match watcher.watch(&now, RecursiveMode::Recursive) {
                Ok(()) => *was = now,
                Err(why) => eprintln!("mcp-host: {} could not be watched again: {why}", now.display()),
              }
            }
            Ok(_) => {}
            Err(why) => eprintln!("mcp-host: {} could not be followed: {why}", dir.display()),
          }
        }

        placed.reread();
      }
    });

    Ok(())
  }
}

/// Every link on the way to `dir`, itself included where it is one: a rename over any of them moves where
/// `dir` really lies.
fn links_on(dir: &Path) -> Vec<PathBuf> {
  dir
    .ancestors()
    .filter(|step| std::fs::symlink_metadata(step).is_ok_and(|meta| meta.file_type().is_symlink()))
    .map(Path::to_path_buf)
    .collect()
}

/// Whether a change concerns a watched folder: one under it, or one in a directory a link on its way stands in
/// that touches the link itself. The rest of such a directory — the deploy tree's neighbours — is no reason to
/// read.
pub fn concerns(paths: &[PathBuf], links: &[PathBuf], folders: &[PathBuf]) -> bool {
  paths.is_empty()
    || paths.iter().any(|path| {
      links.contains(path)
        || !path.parent().is_some_and(|parent| folders.iter().any(|folder| folder == parent))
    })
}
