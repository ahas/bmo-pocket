//! The tool's side of the tool host's call protocol, version 1 (the host's `docs/api.md`, 「The call protocol」):
//! one call handed on standard input and answered on standard output. `word_chain_start` and `word_chain_stop`
//! are carried to the service's control routes, whose address and token the configuration under the record's
//! data root names, and their answer is the tool's.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::config::{Config, Peer};

/// The protocol version this side speaks.
pub const PROTOCOL: u64 = 1;

/// A tool result saying plain text.
pub fn text(said: &str) -> Value {
  json!({ "content": [{ "type": "text", "text": said }], "isError": false })
}

/// A tool result refusing, which the model reads as any other answer.
pub fn refusal(said: &str) -> Value {
  json!({ "content": [{ "type": "text", "text": said }], "isError": true })
}

/// One variable of the turn the call is made in: the run the record's place names — none for a person's turn —
/// over the record's own run variables.
pub fn turn_variable(session: &Value, name: &str) -> Option<String> {
  let from_place = session["place"].as_str().and_then(|place| {
    let run: Value =
      serde_json::from_str(&std::fs::read_to_string(Path::new(place).join("run.json")).ok()?).ok()?;

    if run["person"].as_bool() == Some(true) {
      return None;
    }

    run["env"][name].as_str().map(str::to_owned)
  });

  from_place.or_else(|| session["runVariables"][name].as_str().map(str::to_owned))
}

/// The room a call is for: the one its arguments name, else the turn's own.
fn room(arguments: &Value, session: &Value) -> Option<i64> {
  arguments["chat"]
    .as_i64()
    .or_else(|| turn_variable(session, "BMO_TURN_CHAT").and_then(|chat| chat.trim().parse().ok()))
}

/// What a start's arguments ask the control route for: the robot playing at normal unless `bmo` names another
/// difficulty or `off`.
pub fn start_body(arguments: &Value, chat: i64) -> Result<Value, String> {
  let robot = match arguments["bmo"].as_str() {
    None => Some("normal"),
    Some("off") => None,
    Some(named @ ("easy" | "normal" | "hard")) => Some(named),
    Some(other) => return Err(format!("bmo takes off, easy, normal or hard, not {other}")),
  };
  let mut body = json!({
    "chatId": chat,
    "robot": robot,
    "loanwords": arguments["no_loanwords"].as_bool() != Some(true),
    "manner": arguments["manner"].as_bool() == Some(true),
  });

  if let Some(starter) = arguments["starter"].as_i64() {
    body["starterId"] = json!(starter);
    body["starterName"] = arguments["starter_name"].clone();
  }

  if let Some(seconds) = arguments["seconds"].as_i64() {
    body["turnSeconds"] = json!(seconds);
  }

  if let Some(word) = arguments["word"].as_str() {
    body["opening"] = json!(word);
  }

  Ok(body)
}

/// What a stop's arguments ask the control route for.
pub fn stop_body(arguments: &Value, chat: i64) -> Value {
  let mut body = json!({ "chatId": chat });

  if let Some(player) = arguments["player"].as_i64() {
    body["playerId"] = json!(player);
  }

  body
}

/// One word to the control routes, answered with the line they said, or the error they refused with.
pub async fn ask(control: &Peer, path: &str, body: &Value) -> Result<String, String> {
  let reply = reqwest::Client::new()
    .post(control.url(path))
    .bearer_auth(&control.token)
    .json(body)
    .send()
    .await
    .map_err(|why| format!("the word chain does not answer at {}: {why}", control.base))?;
  let status = reply.status();
  let said: Value = reply.json().await.unwrap_or(Value::Null);

  if status.is_success() {
    Ok(said["line"].as_str().unwrap_or_default().to_owned())
  } else {
    Err(
      said["error"]
        .as_str()
        .map_or_else(|| format!("the word chain answered {status}"), str::to_owned),
    )
  }
}

/// Where the configuration a call's record names stands.
fn configuration(session: &Value) -> PathBuf {
  Config::deployed(Path::new(session["dataRoot"].as_str().unwrap_or_default()))
}

/// One call answered as the protocol answers it; an object that does not read as a call is the error, which the
/// executable says on standard error.
pub async fn answer(handed: &Value) -> Result<Value, String> {
  if handed["protocol"].as_u64() != Some(PROTOCOL) {
    return Err(format!("the call does not read: it speaks no protocol {PROTOCOL}"));
  }

  match handed["kind"].as_str() {
    Some("call") => {}
    Some("claim") => return Ok(json!({ "arguments": null })),
    Some("resources_list") => return Ok(json!({ "resources": [] })),
    Some("resources_read") => return Ok(json!({ "error": "this executable serves no resources" })),
    _ => return Err("the call does not read: it names no kind this executable answers".to_owned()),
  }

  let (entry, arguments, session) = (&handed["entry"], &handed["arguments"], &handed["session"]);
  let Some(chat) = room(arguments, session) else {
    return Ok(refusal("No room is named and no room's turn is going, so no game can be reached."));
  };
  let config = match Config::read(&configuration(session)) {
    Ok(config) => config,
    Err(why) => return Ok(refusal(&format!("ERROR: {why}"))),
  };
  let asked = match entry.as_str() {
    Some("bmo/word_chain_start") => start_body(arguments, chat).map(|body| ("/word-chain/start", body)),
    Some("bmo/word_chain_stop") => Ok(("/word-chain/stop", stop_body(arguments, chat))),
    other => Err(format!("ERROR: this executable holds no entry {}", other.unwrap_or_default())),
  };
  let (path, body) = match asked {
    Ok(asked) => asked,
    Err(why) => return Ok(refusal(&why)),
  };

  Ok(match ask(&config.control(), path, &body).await {
    Ok(line) => text(&line),
    Err(why) => refusal(&why),
  })
}
