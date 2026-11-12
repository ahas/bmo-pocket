//! The console bundle the host serves on its console face (docs/api.md, 「The console face」): the screen's module,
//! stylesheet and words as the build of `console/` wrote them, read once as the host starts, each file named by
//! its bytes, and the manifest naming them and the layer the build was made on.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The bundle's name, its words' namespace, its screen's path and the name a console relays the host under.
pub const NAME: &str = "pocket";

/// The least role a reader holds to open the screen, and so the floor of every route the screen uses.
pub const SCREEN_ROLE: &str = "administrator";

/// The path the bundle's files are answered under.
pub const FILES: &str = "/console-bundle";

/// The files the build of `console/` writes.
const MODULE_FILE: &str = "module.js";
const STYLESHEET_FILE: &str = "style.css";
const WORDS_FILE: &str = "words.json";
const LAYER_FILE: &str = "layer.json";

/// How many hex digits of a file's SHA-256 its name carries.
const DIGEST_DIGITS: usize = 16;

/// The manifest as the console plugin contract shapes it, every field written.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleManifest {
  pub name: String,
  pub title: String,
  pub icon: String,
  pub screen_path: String,
  pub service_name: String,
  pub layer: String,
  pub bundle: String,
  pub stylesheet: Option<String>,
  pub words: String,
  pub api_prefix: String,
  pub role: String,
  pub owner_key: bool,
  pub events: bool,
  pub websocket: bool,
  pub push_kinds: Vec<String>,
  pub calls: Vec<String>,
}

/// A file of the bundle as it is answered: its name under the files' path, its bytes and its media type.
#[derive(Clone, Debug)]
pub struct Served {
  pub name: String,
  pub bytes: Vec<u8>,
  pub kind: &'static str,
}

impl Served {
  /// A file named by its stem, the first hex digits of its bytes' SHA-256 and its ending.
  fn named(stem: &str, ending: &str, kind: &'static str, bytes: Vec<u8>) -> Self {
    let digest: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();

    Self {
      name: format!("{stem}.{}.{ending}", &digest[..DIGEST_DIGITS]),
      bytes,
      kind,
    }
  }

  /// The path of the face it is answered at.
  pub fn path(&self) -> String {
    format!("{FILES}/{}", self.name)
  }
}

/// The built bundle: its module, its stylesheet where the build wrote one, its words and the layer it was built on.
#[derive(Clone, Debug)]
pub struct Bundle {
  pub module: Served,
  pub stylesheet: Option<Served>,
  /// Each locale's tree under the locale's code, as the words file holds it.
  pub words: Vec<u8>,
  /// The version of bmo-ui the bundle was built on, as the layer's bundle build wrote it.
  pub layer: String,
}

/// The layer file as the layer's bundle build writes it.
#[derive(serde::Deserialize)]
struct LayerFile {
  layer: String,
}

// A file of the folder, or none where it does not stand.
fn read_if_standing(path: &Path) -> io::Result<Option<Vec<u8>>> {
  match std::fs::read(path) {
    Ok(bytes) => Ok(Some(bytes)),
    Err(why) if why.kind() == io::ErrorKind::NotFound => Ok(None),
    Err(why) => Err(why),
  }
}

impl Bundle {
  /// The bundle the folder holds; none where it holds no module, no words or no layer, and why where the words
  /// are not one JSON object or the layer file names no layer.
  pub fn read(folder: &Path) -> io::Result<Option<Self>> {
    let (Some(module), Some(words), Some(layer)) = (
      read_if_standing(&folder.join(MODULE_FILE))?,
      read_if_standing(&folder.join(WORDS_FILE))?,
      read_if_standing(&folder.join(LAYER_FILE))?,
    ) else {
      return Ok(None);
    };

    serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&words).map_err(|why| {
      io::Error::new(io::ErrorKind::InvalidData, format!("the words are not one JSON object: {why}"))
    })?;
    let LayerFile { layer } = serde_json::from_slice(&layer).map_err(|why| {
      io::Error::new(io::ErrorKind::InvalidData, format!("the layer file names no layer: {why}"))
    })?;

    Ok(Some(Self {
      module: Served::named("module", "js", "text/javascript; charset=utf-8", module),
      stylesheet: read_if_standing(&folder.join(STYLESHEET_FILE))?
        .map(|bytes| Served::named("style", "css", "text/css; charset=utf-8", bytes)),
      words,
      layer,
    }))
  }

  /// The bundle in the folder, said in the log and left out where it does not stand or does not read, so the host
  /// stands with no screen.
  pub fn found(folder: &Path) -> Option<Self> {
    match Self::read(folder) {
      Ok(Some(bundle)) => Some(bundle),
      Ok(None) => {
        eprintln!(
          "mcp-host: {} holds no console bundle; the console face serves no screen",
          folder.display()
        );
        None
      }
      Err(why) => {
        eprintln!("mcp-host: the console bundle in {} could not be read: {why}", folder.display());
        None
      }
    }
  }

  /// The manifest naming this bundle.
  pub fn manifest(&self) -> ConsoleManifest {
    ConsoleManifest {
      name: NAME.to_owned(),
      title: format!("{NAME}.title.screen"),
      icon: "i-material-symbols-home-repair-service-outline".to_owned(),
      screen_path: format!("/{NAME}"),
      service_name: NAME.to_owned(),
      layer: self.layer.clone(),
      bundle: self.module.path(),
      stylesheet: self.stylesheet.as_ref().map(Served::path),
      words: format!("{FILES}/words"),
      api_prefix: "/".to_owned(),
      role: SCREEN_ROLE.to_owned(),
      owner_key: false,
      events: false,
      websocket: false,
      push_kinds: Vec::new(),
      calls: Vec::new(),
    }
  }

  /// The module or the stylesheet the name names.
  pub fn file(&self, name: &str) -> Option<&Served> {
    std::iter::once(&self.module).chain(&self.stylesheet).find(|served| served.name == name)
  }
}

/// The folder a release's bundle stands in: `console-bundle/` in the folder above the one holding the binary, apart
/// from the screen's sources in `console/`.
pub fn bundle_beside_binary() -> Option<PathBuf> {
  let binary = std::env::current_exe().ok()?;

  Some(binary.parent()?.parent()?.join("console-bundle"))
}
