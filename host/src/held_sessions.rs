//! The sessions the host keeps issued itself (docs/mcp-host.md, 「Held sessions」): one declaration a file in the
//! data root's `tool-host/held/`, `<name>.json`, holding a record as `POST /tool-sessions` takes it without
//! `ttlMs`, and beside it `<name>.token`, which the host writes with the token of the session issued from it.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;

use bmo_tool::{Session, SessionRequest};

use crate::records::{Records, now_ms};

/// The folder of the declarations under the host's own folder of the data root.
pub const HELD: &str = "held";

const DECLARATION: &str = "json";
const TOKEN: &str = "token";

/// Every declaration in `dir` kept issued, and the session of every token file whose declaration no longer stands
/// revoked, its file removed. Answers the sessions revoked, whose kept values the caller ends. A folder that does not
/// stand declares nothing.
pub fn keep(dir: &Path, records: &Records) -> Vec<Arc<Session>> {
  let Ok(listing) = fs::read_dir(dir) else {
    return Vec::new();
  };
  let mut declared = BTreeSet::new();
  let mut tokens = BTreeSet::new();

  for path in listing.filter_map(|entry| entry.ok().map(|entry| entry.path())) {
    let (Some(stem), Some(extension)) = (
      path.file_stem().and_then(|stem| stem.to_str()),
      path.extension().and_then(|extension| extension.to_str()),
    ) else {
      continue;
    };

    if stem.starts_with('.') {
      continue;
    }

    match extension {
      DECLARATION => declared.insert(stem.to_owned()),
      TOKEN => tokens.insert(stem.to_owned()),
      _ => false,
    };
  }

  let mut gone = Vec::new();

  for name in &declared {
    match kept(dir, name, records) {
      Ok(revoked) => gone.extend(revoked),
      Err(why) => eprintln!("mcp-host: the held session {name} is not kept: {why}"),
    }
  }

  for name in tokens.difference(&declared) {
    match dropped(dir, name, records) {
      Ok(revoked) => gone.extend(revoked),
      Err(why) => eprintln!("mcp-host: the held session {name} could not be taken back: {why}"),
    }
  }

  gone
}

// One declaration kept issued: nothing done where its token file names a standing session issued from the record it
// declares, and otherwise the record issued afresh, its token written, and the session the file named before
// revoked, which is answered.
fn kept(dir: &Path, name: &str, records: &Records) -> Result<Option<Arc<Session>>, String> {
  let declaration = dir.join(format!("{name}.{DECLARATION}"));
  let text = fs::read_to_string(&declaration).map_err(|why| format!("{}: {why}", declaration.display()))?;
  let request: SessionRequest = serde_json::from_str(&text)
    .map_err(|why| format!("{} is no session record: {why}", declaration.display()))?;

  if request.ttl_ms.is_some() {
    return Err(format!(
      "{} names ttlMs, and a held session stands while it is declared",
      declaration.display()
    ));
  }

  let token_file = dir.join(format!("{name}.{TOKEN}"));
  let before = fs::read_to_string(&token_file).ok().map(|token| token.trim().to_owned());
  let standing = before.as_deref().and_then(|token| records.get(token));

  if standing.is_some_and(|(_, session)| request.clone().issued_at(session.created_at) == *session) {
    return Ok(None);
  }

  let token = records
    .issue(request.issued_at(now_ms()))
    .map_err(|why| format!("it could not be issued: {why}"))?;

  if let Err(why) = written(dir, name, &token) {
    let _ = records.revoke(&token);

    return Err(format!("its token could not be written into {}: {why}", token_file.display()));
  }

  println!("mcp-host: the held session {name} issued");

  match before {
    Some(token) => records
      .revoke(&token)
      .map(|(_, revoked)| revoked)
      .map_err(|why| format!("the session it held before could not be revoked: {why}")),
    None => Ok(None),
  }
}

// The session of a token file whose declaration no longer stands revoked, and the file removed.
fn dropped(dir: &Path, name: &str, records: &Records) -> Result<Option<Arc<Session>>, String> {
  let token_file = dir.join(format!("{name}.{TOKEN}"));
  let token = fs::read_to_string(&token_file).map_err(|why| format!("{}: {why}", token_file.display()))?;
  let (_, revoked) = records.revoke(token.trim()).map_err(|why| why.to_string())?;

  fs::remove_file(&token_file).map_err(|why| format!("{}: {why}", token_file.display()))?;
  println!("mcp-host: the held session {name} taken back with its declaration");

  Ok(revoked)
}

// The token written into `<name>.token`, readable by the host's user alone, by way of a staged file renamed over it so
// a holder never reads half of it.
fn written(dir: &Path, name: &str, token: &str) -> io::Result<()> {
  let staged = dir.join(format!(".{name}.{TOKEN}.tmp"));

  match fs::remove_file(&staged) {
    Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
    _ => {}
  }

  let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&staged)?;

  file.write_all(format!("{token}\n").as_bytes())?;
  file.sync_all()?;
  fs::rename(&staged, dir.join(format!("{name}.{TOKEN}")))
}
