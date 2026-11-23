//! The relay a personal tool's call is handed to (docs/mcp-host.md, 「Personal tools」; docs/api.md, 「The
//! runner」): started by the host as any tool's executable is, the call on its standard input, it asks the owner's
//! runner over the passage the session's record names and answers what the runner's run of the tool answered, on
//! its standard output, its errors and its exit code. It holds the passage open until the runner answers, so the
//! relay ended — its caller gone — closes the passage, and the runner ends the tool's process. The secrets a call is
//! handed are read here, at home, and travel on the passage's first line alone: never on a command line, in a log
//! or in what the relay says.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use base64::Engine;
use bmo_tool::Runner;
use bmo_tool::manifest::secret_name;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// The most bytes one secret handed to a call holds.
pub const SECRET_BYTES: u64 = 64 * 1024;

/// What the host tells the relay of one call, beside the call itself: the owner and the tool, the version the host's
/// copy of it is placed as, its run words, the runner the session's record names, the addresses the call may reach,
/// and the names of the secrets it is handed with the folder they are read from. Names alone: no value is ever told
/// the relay on its command line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
  pub owner: String,
  pub tool: String,
  pub version: String,
  pub run: Vec<String>,
  pub runner: Runner,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub network: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub secrets: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub secret_space: Option<PathBuf>,
}

/// What the passage is asked, on its first line: the owner, the tool and its version, its run words, the call as the
/// tool's executable is handed it, the addresses it may reach, and the secrets it is handed for this call alone, each
/// file's bytes in base64 by its name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Passage {
  pub owner: String,
  pub tool: String,
  pub version: String,
  pub run: Vec<String>,
  pub call: Value,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub network: Vec<String>,
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub secrets: BTreeMap<String, String>,
}

/// The secrets named, read from the owner's secret space, each file's bytes in base64 by its name; or why not, naming
/// the secret and never what a file holds. A name that is no plain file's, one that stands as no regular file — a link
/// or a folder among them — and a file over [`SECRET_BYTES`] are refused.
pub fn secrets(space: Option<&Path>, names: &[String]) -> Result<BTreeMap<String, String>, String> {
  let mut handed = BTreeMap::new();

  if names.is_empty() {
    return Ok(handed);
  }

  let Some(space) = space else {
    return Err("the call names secrets, and no secret space is named to hand them from".to_owned());
  };

  for name in names {
    if !secret_name(name) {
      return Err(format!("{name:?} is no secret's name"));
    }

    let path = space.join(name);
    let missing = || {
      format!(
        "the secret {name} this app needs is not in the owner's secret space; save it under that name first"
      )
    };
    let standing = std::fs::symlink_metadata(&path).map_err(|_| missing())?;

    if !standing.file_type().is_file() {
      return Err(missing());
    }

    if standing.len() > SECRET_BYTES {
      return Err(format!(
        "the secret {name} holds more than {SECRET_BYTES} bytes, the most a call is handed"
      ));
    }

    let bytes =
      std::fs::read(&path).map_err(|why| format!("the secret {name} could not be read: {}", why.kind()))?;

    handed.insert(name.clone(), base64::engine::general_purpose::STANDARD.encode(bytes));
  }

  Ok(handed)
}

/// One line the passage answers: a line the tool wrote on its standard error; its end, with the code it ended with,
/// none where a signal ended it, and what it wrote on its standard output; or why the runner did not run it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Answered {
  Said { line: String },
  End { code: Option<i32>, out: String },
  Refused { why: String },
}

impl Answered {
  /// The answer as one line of the passage.
  pub fn line(&self) -> String {
    let mut line = serde_json::to_string(self).unwrap_or_default();

    line.push('\n');
    line
  }
}

/// The passage's two halves, and the runner's process where the road starts one, ended as the asker ends.
pub(crate) struct Opened {
  pub(crate) reading: Box<dyn AsyncRead + Unpin + Send>,
  pub(crate) writing: Box<dyn AsyncWrite + Unpin + Send>,
  pub(crate) _child: Option<tokio::process::Child>,
}

/// The passage the runner's road opens.
pub(crate) async fn open(runner: &Runner) -> Result<Opened, String> {
  match runner {
    Runner::Socket(path) => {
      let stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|why| format!("the runner's socket {} could not be reached: {why}", path.display()))?;
      let (reading, writing) = stream.into_split();

      Ok(Opened {
        reading: Box::new(reading),
        writing: Box::new(writing),
        _child: None,
      })
    }
    Runner::Command(words) => {
      let (program, args) = words.split_first().ok_or("the runner's command names nothing to run")?;
      let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(|why| format!("the runner {program} could not be started: {why}"))?;
      let (Some(writing), Some(reading)) = (child.stdin.take(), child.stdout.take()) else {
        return Err("the runner gave no input or output".to_owned());
      };

      Ok(Opened {
        reading: Box::new(reading),
        writing: Box::new(writing),
        _child: Some(child),
      })
    }
  }
}

/// One call relayed: the call read whole from `input`, the passage asked, each line the tool wrote on its errors
/// said on `err`, and what it wrote on its output put on `out`; the code the relay ends with, the tool's own where
/// it ended with one, and 1 where it did not end, ended by a signal or was refused.
pub async fn relay(
  request: Request,
  mut input: impl AsyncRead + Unpin,
  mut out: impl AsyncWrite + Unpin,
  mut err: impl AsyncWrite + Unpin,
) -> u8 {
  let mut say = async |line: String| {
    let _ = err.write_all(format!("{line}\n").as_bytes()).await;
    let _ = err.flush().await;
  };
  let mut handed = Vec::new();

  if let Err(why) = input.read_to_end(&mut handed).await {
    say(format!("the call could not be read: {why}")).await;
    return 1;
  }

  let call: Value = match serde_json::from_slice(&handed) {
    Ok(call) => call,
    Err(why) => {
      say(format!("the call does not read: {why}")).await;
      return 1;
    }
  };
  // Read before the runner is asked, so a secret missing stops the call here, naming it.
  let secrets = match secrets(request.secret_space.as_deref(), &request.secrets) {
    Ok(secrets) => secrets,
    Err(why) => {
      say(why).await;
      return 1;
    }
  };
  let passage = Passage {
    owner: request.owner,
    tool: request.tool,
    version: request.version,
    run: request.run,
    call,
    network: request.network,
    secrets,
  };
  let mut opened = match open(&request.runner).await {
    Ok(opened) => opened,
    Err(why) => {
      say(why).await;
      return 1;
    }
  };
  let mut asked = serde_json::to_string(&passage).unwrap_or_default();

  asked.push('\n');

  if let Err(why) = opened.writing.write_all(asked.as_bytes()).await {
    say(format!("the runner could not be asked: {why}")).await;
    return 1;
  }

  let _ = opened.writing.flush().await;
  let mut lines = BufReader::new(opened.reading).lines();

  // The writing half stays open until the answer: the passage closing is what tells the runner its caller is gone.
  loop {
    let line = match lines.next_line().await {
      Ok(Some(line)) => line,
      Ok(None) => {
        say("the runner ended without an answer".to_owned()).await;
        return 1;
      }
      Err(why) => {
        say(format!("the runner's answer could not be read: {why}")).await;
        return 1;
      }
    };

    match serde_json::from_str::<Answered>(&line) {
      Ok(Answered::Said { line }) => say(line).await,
      Ok(Answered::End { code, out: written }) => {
        let _ = out.write_all(written.as_bytes()).await;
        let _ = out.flush().await;

        return match code {
          Some(0) => 0,
          Some(code) => u8::try_from(code).ok().filter(|code| *code != 0).unwrap_or(1),
          None => {
            say("the tool was ended by a signal".to_owned()).await;
            1
          }
        };
      }
      Ok(Answered::Refused { why }) => {
        say(format!("the runner did not run it: {why}")).await;
        return 1;
      }
      Err(why) => {
        say(format!("the runner answered what does not read: {why}")).await;
        return 1;
      }
    }
  }
}
