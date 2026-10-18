//! The placed tools' catalogue (docs/mcp-host.md, 「The catalogue」): read from the deploy tree's `tools/`,
//! each folder's `.placed/` alone, and read again whole once the changes under it settle, so a tool placed,
//! replaced or taken away is served from the next call with nothing restarted.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub use bmo_tool::manifest::PLACED;
use bmo_tool::manifest::{MANIFEST, Manifest, SHARED};
use notify::event::ModifyKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// A tool as it is placed: its folder's name, where it is placed, and its manifest.
#[derive(Debug)]
pub struct PlacedTool {
  pub folder: String,
  pub dir: PathBuf,
  pub manifest: Manifest,
}

/// Every placed tool and shared helper, as one reading of `tools/` found them.
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

// What is wrong with a manifest's schemas as JSON Schema, where anything is.
fn schema_faults(manifest: &Manifest) -> Option<String> {
  manifest.entries.iter().find_map(|entry| {
    std::iter::once(&entry.input_schema)
      .chain(&entry.output_schema)
      .find_map(|schema| jsonschema::validator_for(schema).err())
      .map(|why| format!("{}: its schema is not a JSON Schema: {why}", entry.full_name()))
  })
}

impl Catalogue {
  /// The catalogue `tools/` holds, and why each tool left out was left out.
  pub fn read(dir: &Path) -> (Self, Vec<String>) {
    let mut catalogue = Self::default();
    let mut faults = Vec::new();
    let mut held = BTreeSet::new();

    for (folder, path) in folders(dir) {
      if folder == SHARED {
        for (name, helper) in folders(&path) {
          let placed = helper.join(PLACED);

          if !placed.join(MANIFEST).exists() {
            continue;
          }

          match Manifest::read(&placed, &name) {
            Ok(manifest) if manifest.helper.is_some() => {
              catalogue.helpers.insert(
                name.clone(),
                Arc::new(PlacedTool {
                  folder: name,
                  dir: placed,
                  manifest,
                }),
              );
            }
            Ok(_) => {
              faults.push(format!("tools/{SHARED}/{name}: a shared helper's manifest has no [helper]"))
            }
            Err(why) => faults.push(format!("tools/{SHARED}/{name}: {why}")),
          }
        }

        continue;
      }

      let placed = path.join(PLACED);

      if folder.starts_with('_') || !placed.join(MANIFEST).exists() {
        continue;
      }

      let manifest = match Manifest::read(&placed, &folder) {
        Ok(manifest) if manifest.helper.is_some() => {
          faults.push(format!("tools/{folder}: a shared helper stands under tools/{SHARED}/"));
          continue;
        }
        Ok(manifest) => manifest,
        Err(why) => {
          faults.push(format!("tools/{folder}: {why}"));
          continue;
        }
      };

      if let Some(why) = schema_faults(&manifest) {
        faults.push(format!("tools/{folder}: {why}"));
        continue;
      }

      let names: Vec<String> = manifest.entries.iter().map(|entry| entry.full_name()).collect();

      if let Some(taken) = names.iter().find(|name| held.contains(*name)) {
        faults.push(format!("tools/{folder}: {taken} is another tool's already"));
        continue;
      }

      held.extend(names);
      catalogue.tools.push(Arc::new(PlacedTool {
        folder,
        dir: placed,
        manifest,
      }));
    }

    (catalogue, faults)
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
}

/// Whether a change under `tools/` can move what is placed: a file or folder made, written, renamed or taken
/// away. Opening and reading one — the host reading a manifest, a tool run reading its own files — moves
/// nothing, and is never a reason to read again.
pub fn moves(kind: &EventKind) -> bool {
  !matches!(kind, EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(_)))
}

/// The catalogue the host serves from, and where it is read.
pub struct Placed {
  /// The deploy tree's `tools/`; none where the host serves no tool.
  pub dir: Option<PathBuf>,
  /// Where each call's call-back socket and each helper's socket are bound.
  pub sockets: PathBuf,
  current: RwLock<Arc<Catalogue>>,
}

impl Placed {
  pub fn new(dir: Option<PathBuf>, sockets: PathBuf) -> Self {
    Self {
      dir,
      sockets,
      current: RwLock::new(Arc::new(Catalogue::default())),
    }
  }

  /// The catalogue as the last reading left it.
  pub fn current(&self) -> Arc<Catalogue> {
    self.current.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
  }

  /// `tools/` read again whole and put in place at once, each tool left out said in the log.
  pub fn reread(&self) {
    let Some(dir) = &self.dir else {
      return;
    };
    let (catalogue, faults) = Catalogue::read(dir);

    for fault in &faults {
      eprintln!("mcp-host: left out {fault}");
    }

    eprintln!(
      "mcp-host: read {}: {} tools and {} shared helpers placed",
      dir.display(),
      catalogue.tools.len(),
      catalogue.helpers.len()
    );
    *self.current.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(catalogue);
  }

  /// `tools/` read now and again once every run of changes under it has settled for `settle`, for as long as the
  /// host runs. A `tools/` taken away whole is made again and watched anew, and one whose real path moves — a
  /// link on its path flipped onto another release — is watched where it now lies and read there.
  pub fn watch(self: &Arc<Self>, settle: Duration) -> notify::Result<()> {
    let Some(dir) = self.dir.clone() else {
      return Ok(());
    };

    std::fs::create_dir_all(&dir).map_err(notify::Error::io)?;

    let links = links_on(&dir);
    let rooms: Vec<PathBuf> =
      links.iter().filter_map(|link| link.parent().map(Path::to_path_buf)).collect();
    let mut real = std::fs::canonicalize(&dir).map_err(notify::Error::io)?;
    let (changed, mut changes) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut watcher: RecommendedWatcher = {
      let (links, rooms) = (links.clone(), rooms.clone());

      notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| moves(&event.kind) && concerns(&event.paths, &links, &rooms)) {
          let _ = changed.send(());
        }
      })?
    };

    watcher.watch(&real, RecursiveMode::Recursive)?;

    for room in &rooms {
      watcher.watch(room, RecursiveMode::NonRecursive)?;
    }

    self.reread();

    let placed = self.clone();

    tokio::spawn(async move {
      while changes.recv().await.is_some() {
        loop {
          match tokio::time::timeout(settle, changes.recv()).await {
            Ok(Some(())) => {}
            Ok(None) => return,
            Err(_) => break,
          }
        }

        // A `tools/` taken away whole is made again, and its watch went with it.
        let remade = !dir.is_dir();

        if remade && let Err(why) = std::fs::create_dir_all(&dir) {
          eprintln!("mcp-host: {} could not be made again: {why}", dir.display());
        }

        match std::fs::canonicalize(&dir) {
          Ok(now) if remade || now != real => {
            let _ = watcher.unwatch(&real);

            match watcher.watch(&now, RecursiveMode::Recursive) {
              Ok(()) => real = now,
              Err(why) => eprintln!("mcp-host: {} could not be watched again: {why}", now.display()),
            }
          }
          Ok(_) => {}
          Err(why) => eprintln!("mcp-host: {} could not be followed: {why}", dir.display()),
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

/// Whether a change concerns `tools/`: one under it, or one in a directory a link on its way stands in that
/// touches the link itself. The rest of such a directory — the deploy tree's neighbours — is no reason to read.
pub fn concerns(paths: &[PathBuf], links: &[PathBuf], rooms: &[PathBuf]) -> bool {
  paths.is_empty()
    || paths.iter().any(|path| {
      links.contains(path) || !path.parent().is_some_and(|parent| rooms.iter().any(|room| room == parent))
    })
}
