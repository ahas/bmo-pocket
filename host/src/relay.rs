//! The relay a personal tool's call is handed to (docs/mcp-host.md, 「Personal tools」; docs/api.md, 「The
//! runner」): started by the host as any tool's executable is, the call on its standard input, it asks the owner's
//! runner over the passage the session's record names and answers what the runner's run of the tool answered, on
//! its standard output, its errors and its exit code. It holds the passage open until the runner answers, so the
//! relay ended — its caller gone — closes the passage, and the runner ends the tool's process.

use std::process::Stdio;

use bmo_tool::Runner;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// What the host tells the relay of one call, beside the call itself: the owner and the tool, the version the host's
/// copy of it is placed as, its run words, and the runner the session's record names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
  pub owner: String,
  pub tool: String,
  pub version: String,
  pub run: Vec<String>,
  pub runner: Runner,
}

/// What the passage is asked, on its first line: the owner, the tool and its version, its run words, and the call
/// as the tool's executable is handed it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Passage {
  pub owner: String,
  pub tool: String,
  pub version: String,
  pub run: Vec<String>,
  pub call: Value,
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

// The passage's two halves, and the runner's process where the road starts one, ended as the relay ends.
struct Opened {
  reading: Box<dyn AsyncRead + Unpin + Send>,
  writing: Box<dyn AsyncWrite + Unpin + Send>,
  _child: Option<tokio::process::Child>,
}

async fn open(runner: &Runner) -> Result<Opened, String> {
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
  let passage = Passage {
    owner: request.owner,
    tool: request.tool,
    version: request.version,
    run: request.run,
    call,
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
