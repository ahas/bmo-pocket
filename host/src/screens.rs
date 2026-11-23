//! The screens of personal tools (docs/mcp-host.md, 「Personal screens」; docs/api.md, 「Personal screens」 and
//! 「`[screen]`」): a screen's words, layer and index read from the host's copy of its tool, the screen as a console
//! attaches it for its owner, and the asks the owner's runner is put for the screen — whether it answers, and one file
//! of the screen's folder.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine;
use bmo_pocket_client::PersonalScreen;
use bmo_tool::Runner;
use bmo_tool::manifest::{
  Manifest, SCREEN_FOLDER, SCREEN_ICON, SCREEN_INDEX, SCREEN_LAYER, SCREEN_MODULE, SCREEN_STYLESHEET,
  SCREEN_WORDS, inside, screen_name, screen_path,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::relay::{Opened, open};

/// What the host's copy of a tool holds of its screen: its words, the layer it was built on, its index of files by
/// path with each one's SHA-256, and the digest of the index's bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenFiles {
  pub words: Map<String, Value>,
  pub layer: String,
  pub index: BTreeMap<String, String>,
  pub digest: String,
}

// A SHA-256 written as the index writes one: 64 lowercase hexadecimal digits.
fn sha256_text(text: &str) -> bool {
  text.len() == 64 && text.chars().all(|one| one.is_ascii_digit() || ('a'..='f').contains(&one))
}

/// The SHA-256 of some bytes in lowercase hexadecimal.
pub fn sha256_hex(bytes: &[u8]) -> String {
  format!("{:x}", Sha256::digest(bytes))
}

fn read_json(dir: &Path, file: &str) -> Result<(Vec<u8>, Value), String> {
  let path = dir.join(SCREEN_FOLDER).join(file);
  let bytes = std::fs::read(&path).map_err(|why| format!("{SCREEN_FOLDER}/{file}: {why}"))?;
  let value =
    serde_json::from_slice(&bytes).map_err(|why| format!("{SCREEN_FOLDER}/{file} does not read: {why}"))?;

  Ok((bytes, value))
}

/// The screen of the tool `tool` its folder `dir` holds, held to its rules: a layer named; words that are an object
/// of locales, each tree holding the screen's name alone and its title under it; and an index naming the module and
/// every file by a path inside the folder with a SHA-256. Or why it does not read.
pub fn read_screen(dir: &Path, tool: &str) -> Result<ScreenFiles, String> {
  let name = screen_name(tool);
  let (_, layer) = read_json(dir, SCREEN_LAYER)?;
  let layer = layer
    .get("layer")
    .and_then(Value::as_str)
    .filter(|layer| !layer.trim().is_empty())
    .ok_or_else(|| format!("{SCREEN_FOLDER}/{SCREEN_LAYER} names no layer"))?
    .to_owned();
  let (_, words) = read_json(dir, SCREEN_WORDS)?;
  let Value::Object(words) = words else {
    return Err(format!("{SCREEN_FOLDER}/{SCREEN_WORDS} is no object of locales"));
  };

  if words.is_empty() {
    return Err(format!("{SCREEN_FOLDER}/{SCREEN_WORDS} holds no locale"));
  }

  for (locale, tree) in &words {
    let Some(tree) = tree.as_object() else {
      return Err(format!("{SCREEN_FOLDER}/{SCREEN_WORDS}: the locale {locale} holds no tree"));
    };

    if tree.keys().any(|key| key != &name) {
      return Err(format!(
        "{SCREEN_FOLDER}/{SCREEN_WORDS}: the locale {locale} holds words beside `{name}`"
      ));
    }

    if !tree.get(&name).and_then(|own| own.get("title")).is_some_and(Value::is_string) {
      return Err(format!("{SCREEN_FOLDER}/{SCREEN_WORDS}: the locale {locale} names no `{name}.title`"));
    }
  }

  let (bytes, index) = read_json(dir, SCREEN_INDEX)?;
  let Value::Object(listed) = index else {
    return Err(format!("{SCREEN_FOLDER}/{SCREEN_INDEX} is no object of files"));
  };
  let mut index = BTreeMap::new();

  for (path, sha) in listed {
    let Some(sha) = sha.as_str().filter(|sha| sha256_text(sha)) else {
      return Err(format!("{SCREEN_FOLDER}/{SCREEN_INDEX}: {path} has no SHA-256"));
    };

    if !inside(&path) || path == SCREEN_INDEX {
      return Err(format!("{SCREEN_FOLDER}/{SCREEN_INDEX}: {path} is no file inside the screen's folder"));
    }

    index.insert(path, sha.to_owned());
  }

  if !index.contains_key(SCREEN_MODULE) {
    return Err(format!("{SCREEN_FOLDER}/{SCREEN_INDEX} names no {SCREEN_MODULE}"));
  }

  Ok(ScreenFiles {
    words,
    layer,
    index,
    digest: sha256_hex(&bytes),
  })
}

/// The screen as the app face lists it, its files' paths under the digest of its index.
pub fn listed(tool: &str, version: &str, manifest: &Manifest, files: ScreenFiles) -> PersonalScreen {
  let name = screen_name(tool);
  let under = |file: &str| format!("/screens/{tool}/{}/{file}", files.digest);

  PersonalScreen {
    tool: tool.to_owned(),
    version: version.to_owned(),
    title: format!("{name}.title"),
    name,
    screen_path: screen_path(tool),
    icon: manifest
      .screen
      .as_ref()
      .and_then(|screen| screen.icon.clone())
      .unwrap_or_else(|| SCREEN_ICON.to_owned()),
    layer: files.layer,
    module: under(SCREEN_MODULE),
    stylesheet: files.index.contains_key(SCREEN_STYLESHEET).then(|| under(SCREEN_STYLESHEET)),
    words: files.words,
    digest: files.digest,
  }
}

// One ask of the runner answered by its first line that is not a line of its errors; a refusal is its reason.
async fn ask(runner: &Runner, line: &Value) -> Result<Value, String> {
  let Opened {
    reading,
    mut writing,
    _child,
  } = open(runner).await?;
  let mut asked = line.to_string();

  asked.push('\n');
  writing
    .write_all(asked.as_bytes())
    .await
    .map_err(|why| format!("the runner could not be asked: {why}"))?;
  let _ = writing.flush().await;

  let mut lines = BufReader::new(reading).lines();

  loop {
    let line = match lines.next_line().await {
      Ok(Some(line)) => line,
      Ok(None) => return Err("the runner ended without an answer".to_owned()),
      Err(why) => return Err(format!("the runner's answer could not be read: {why}")),
    };
    let answered: Value = serde_json::from_str(&line)
      .map_err(|why| format!("the runner answered what does not read: {why}"))?;

    match answered.get("t").and_then(Value::as_str) {
      Some("said") => {}
      Some("refused") => {
        return Err(format!(
          "the runner refused: {}",
          answered.get("why").and_then(Value::as_str).unwrap_or("it gave no reason")
        ));
      }
      _ => return Ok(answered),
    }
  }
}

/// Whether the owner's runner answers: it is asked which versions it keeps.
pub async fn runner_answers(runner: &Runner, owner: &str) -> bool {
  let asked = json!({ "owner": owner, "versions": {} });

  ask(runner, &asked).await.is_ok_and(|answered| answered["t"] == "versions")
}

/// One file of a version's screen folder, read on the owner's machine and answered only where its bytes have the
/// SHA-256 `sha`; or why not.
pub async fn screen_file(
  runner: &Runner,
  owner: &str,
  tool: &str,
  version: &str,
  path: &str,
  sha: &str,
) -> Result<Vec<u8>, String> {
  let asked = json!({ "owner": owner, "file": { "tool": tool, "version": version, "path": path } });
  let answered = ask(runner, &asked).await?;
  let text = match (answered["t"].as_str(), answered["bytes"].as_str()) {
    (Some("file"), Some(text)) => text,
    _ => return Err(format!("the runner answered no file for {path}")),
  };
  let bytes = base64::engine::general_purpose::STANDARD
    .decode(text)
    .map_err(|why| format!("the runner answered bytes that do not read for {path}: {why}"))?;

  if sha256_hex(&bytes) != sha {
    return Err(format!("the runner answered other bytes for {path} than the screen's index names"));
  }

  Ok(bytes)
}

/// The media type a screen's file is answered as, by its extension.
pub fn media_type(path: &str) -> &'static str {
  match Path::new(path).extension().and_then(|extension| extension.to_str()) {
    Some("js" | "mjs") => "text/javascript; charset=utf-8",
    Some("css") => "text/css; charset=utf-8",
    Some("json") => "application/json",
    Some("svg") => "image/svg+xml",
    Some("png") => "image/png",
    Some("webp") => "image/webp",
    Some("woff2") => "font/woff2",
    _ => "application/octet-stream",
  }
}
