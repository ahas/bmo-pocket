//! The host's console face (docs/api.md, 「The console face」): bmo-chat's `bmo-console-face` built from the
//! host's declaration in `console/declaration.json` and the folder the build of `console/` wrote.

use std::path::{Path, PathBuf};

use bmo_console_face::{BundleFolder, ConsoleFace, ConsoleFaceError};

/// The manifest fields the host decides, as the declaration file holds them.
pub const DECLARATION: &str = include_str!("../../console/declaration.json");

/// The files the build of `console/` writes that the manifest names.
const MODULE_FILE: &str = "module.js";
const STYLESHEET_FILE: &str = "style.css";
const WORDS_FILE: &str = "words.json";

/// The face of the host's declaration over the bundle in the folder, guarded by the console token; why where the
/// folder does not read or the manifest does not keep the contract.
pub fn face(folder: &Path, token: &str) -> Result<ConsoleFace, ConsoleFaceError> {
  let bundle = BundleFolder::new(folder, MODULE_FILE, WORDS_FILE).with_stylesheet(STYLESHEET_FILE);

  ConsoleFace::new(DECLARATION, &bundle, token)
}

/// The folder a release's bundle stands in: `console-bundle/` in the folder above the one holding the binary, apart
/// from the screen's sources in `console/`.
pub fn bundle_beside_binary() -> Option<PathBuf> {
  let binary = std::env::current_exe().ok()?;

  Some(binary.parent()?.parent()?.join("console-bundle"))
}
