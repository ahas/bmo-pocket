//! The session records the host serves by (docs/mcp-host.md, 「The session record」): one file per
//! record, readable by the owner alone and named by the SHA-256 of its token, read back at start so
//! a restart costs no session its tools. The token itself is never written down.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use bmo_tool::Session;
use sha2::{Digest, Sha256};

pub struct Records {
  dir: PathBuf,
  held: RwLock<HashMap<String, Arc<Session>>>,
}

/// What a record is known by outside the host — its file name — where its token must never be: the
/// SHA-256 of the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordId(pub String);

fn digest(token: &str) -> String {
  Sha256::digest(token.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The present moment, in epoch milliseconds, which a record's deadline is held to.
pub fn now_ms() -> u64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64)
}

// Whether a run's record has outlived its run.
fn lapsed(session: &Session, now: u64) -> bool {
  session.deadline.is_some_and(|deadline| deadline <= now)
}

impl Records {
  /// The records in `dir`, made where it is not. A file that is not a record is left where it is and
  /// named on standard error; a lapsed one is dropped.
  pub fn load(dir: &Path) -> io::Result<Self> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;

    let records = Self {
      dir: dir.to_owned(),
      held: RwLock::new(HashMap::new()),
    };
    let now = now_ms();

    for entry in fs::read_dir(dir)? {
      let path = entry?.path();
      let Some(hash) = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".json"))
      else {
        continue;
      };

      match fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Session>(&text).ok())
      {
        Some(session) if lapsed(&session, now) => {
          let _ = fs::remove_file(&path);
        }
        Some(session) => {
          records
            .held
            .write()
            .expect("the records' lock is never poisoned")
            .insert(hash.to_owned(), Arc::new(session));
        }
        None => eprintln!("mcp-host: {} is not a session record and is left as it stands", path.display()),
      }
    }

    Ok(records)
  }

  fn file(&self, hash: &str) -> PathBuf {
    self.dir.join(format!("{hash}.json"))
  }

  /// A record kept, and the fresh token it is known by from now on.
  pub fn issue(&self, session: Session) -> io::Result<String> {
    let token: String = rand::random::<[u8; 32]>().iter().map(|byte| format!("{byte:02x}")).collect();
    let hash = digest(&token);
    let staged = self.dir.join(format!(".{hash}.json.tmp"));
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&staged)?;

    file.write_all(&serde_json::to_vec(&session)?)?;
    file.sync_all()?;
    fs::rename(&staged, self.file(&hash))?;
    self
      .held
      .write()
      .expect("the records' lock is never poisoned")
      .insert(hash, Arc::new(session));

    Ok(token)
  }

  /// A record taken back, the id it was known by, and the session it held, whose kept values its taker
  /// ends; a token that names none is already taken back.
  pub fn revoke(&self, token: &str) -> io::Result<(RecordId, Option<Arc<Session>>)> {
    let hash = digest(token);
    let gone = self.held.write().expect("the records' lock is never poisoned").remove(&hash);

    match fs::remove_file(self.file(&hash)) {
      Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
      _ => Ok((RecordId(hash), gone)),
    }
  }

  /// The record a token names and its id, where it names one that has not lapsed.
  pub fn get(&self, token: &str) -> Option<(RecordId, Arc<Session>)> {
    let hash = digest(token);
    let found = self.held.read().expect("the records' lock is never poisoned").get(&hash).cloned()?;

    if lapsed(&found, now_ms()) {
      // A lapsed record's kept values end apart from the call that found it lapsed.
      if let Ok((_, Some(gone))) = self.revoke(token) {
        tokio::spawn(async move { gone.kept.close().await });
      }

      return None;
    }

    Some((RecordId(hash), found))
  }

  /// Every lapsed record dropped, so a collector that died leaves nothing standing, with the ids they were
  /// known by and the sessions they held, whose kept values the caller ends.
  pub fn prune(&self) -> Vec<(RecordId, Arc<Session>)> {
    let now = now_ms();
    let lapsed_ones: Vec<String> = self
      .held
      .read()
      .expect("the records' lock is never poisoned")
      .iter()
      .filter(|(_, session)| lapsed(session, now))
      .map(|(hash, _)| hash.clone())
      .collect();
    let mut gone = Vec::new();

    for hash in lapsed_ones {
      let session = self.held.write().expect("the records' lock is never poisoned").remove(&hash);
      let _ = fs::remove_file(self.file(&hash));

      if let Some(session) = session {
        gone.push((RecordId(hash), session));
      }
    }

    gone
  }
}
