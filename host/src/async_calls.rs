//! Asynchronous calls (`docs/api.md`, 「Asynchronous calls」): a session's call kept and run by the host itself,
//! its answer kept when it comes, and the caller's agent woken through the receiver. A caller's shell asks here
//! and writes nothing: the record, the timer and the wake stand with the service.

use std::collections::HashSet;
use std::hash::{BuildHasher as _, RandomState};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bmo_tool::Session;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::{Request, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::net::TcpStream;

use crate::error::HostError;
use crate::handler::{Host, Refusal};

/// The folder the calls stand under, inside the data root.
pub const FOLDER: &str = "use";
/// The file a call's request is kept in.
pub const REQUEST: &str = "request.json";
/// The file a call's answer is kept in.
pub const ANSWER: &str = "answer.json";
/// The file a call's end is kept in.
pub const END: &str = "end.json";

/// The sort every wake is queued under.
pub const SORT: &str = "use-ended";

// The longest summary a wake carries, in characters.
const SUMMARY_CHARS: usize = 160;

/// A call as the caller asked it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asked {
  /// The tool's full name, `<namespace>/<name>`.
  pub tool: String,
  /// The tool's own arguments.
  #[serde(default)]
  pub arguments: Map<String, Value>,
  /// What to hand back with the answer when it comes.
  #[serde(default)]
  pub memory: Option<String>,
  /// The room the wake is for, else the receiver's own choice.
  #[serde(default)]
  pub room: Option<i64>,
  /// The agent the wake is for.
  #[serde(default)]
  pub agent: Option<String>,
  /// The road the caller's turn was read on.
  #[serde(default)]
  pub via: Option<String>,
}

/// A call as it was asked, kept before it runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
  pub id: String,
  /// The session the call was asked under, by the record the token names: the same session alone reads it back.
  pub session: String,
  pub tool: String,
  pub reached_by: String,
  pub arguments: Map<String, Value>,
  pub memory: Option<String>,
  pub room: Option<i64>,
  pub agent: Option<String>,
  pub via: Option<String>,
  pub started_at: i64,
}

/// How a call ended, and whether its wake was queued.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct End {
  pub ended_at: i64,
  pub state: String,
  pub summary: String,
  pub wake: Value,
}

/// The calls the host keeps for its sessions: the data root they stand under, and the ids running now.
pub struct AsyncCalls {
  data_root: PathBuf,
  running: Mutex<HashSet<String>>,
}

impl AsyncCalls {
  /// The calls kept under `data_root`, none running yet.
  pub fn new(data_root: PathBuf) -> Self {
    Self {
      data_root,
      running: Mutex::new(HashSet::new()),
    }
  }

  fn folder(&self) -> PathBuf {
    self.data_root.join(FOLDER)
  }

  fn place(&self, id: &str) -> PathBuf {
    self.folder().join(id)
  }

  /// The call `asked` started: its request kept, its run under way, and the id and state answered. The tool is
  /// resolved first, so an unknown tool or one the session does not hold is refused before anything is kept.
  pub async fn start(
    self: &Arc<Self>,
    host: Arc<Host>,
    session: Arc<Session>,
    under: String,
    asked: Asked,
  ) -> Result<Value, HostError> {
    let Some(found) = host.find_full(&session, &asked.tool, true) else {
      return Err(HostError::UnknownEntry(asked.tool));
    };
    let id = new_id(&asked.tool);
    let request = Kept {
      id: id.clone(),
      session: under,
      tool: asked.tool,
      reached_by: found.reached_by().name().to_owned(),
      arguments: asked.arguments.clone(),
      memory: asked.memory,
      room: asked.room,
      agent: asked.agent,
      via: asked.via,
      started_at: now_ms(),
    };
    let place = self.place(&id);

    keep(&place.join(REQUEST), &request)?;
    self.running.lock().expect("the running calls are held").insert(id.clone());

    let calls = Arc::clone(self);
    let arguments = Value::Object(asked.arguments);
    let under = id.clone();
    let kept = place.clone();

    tokio::spawn(async move {
      let started = Instant::now();
      let run = host.attempt(&found, &session, arguments).await;
      let (answer, state, summary) = match run {
        Ok(result) => {
          let summary = answered_summary(&request.tool, &result);
          (result, "done", summary)
        }
        Err(refusal) => {
          let why = refusal.said().to_owned();
          let code = match &refusal {
            Refusal::Misfit(_) => "arguments-misfit",
            Refusal::Denied(_) => "grant-refused",
            Refusal::Failed(_) => "tool-failed",
          };
          let summary = line(&format!("{} was refused: {code}: {}", request.tool, first_line(&why)));
          (json!({ "code": code, "error": why }), "refused", summary)
        }
      };
      let spent = started.elapsed().as_millis();

      if let Err(why) = keep(&kept.join(ANSWER), &answer) {
        eprintln!("mcp-host: the answer of `{under}` could not be kept: {why} (after {spent} ms)");
      }

      let wake = calls.awake(&request, state, &summary).await;
      let end = End {
        ended_at: now_ms(),
        state: state.to_owned(),
        summary,
        wake: match &wake {
          Ok(instruction) => json!({ "id": instruction }),
          Err(why) => json!({ "error": why }),
        },
      };

      if let Err(why) = keep(&kept.join(END), &end) {
        eprintln!("mcp-host: the end of `{under}` could not be kept: {why}");
      }

      calls.running.lock().expect("the running calls are held").remove(&under);
    });

    Ok(json!({ "id": id, "state": "running", "record": place.display().to_string() }))
  }

  /// Every call kept, oldest first, each as `one` answers it.
  pub fn list(&self) -> Vec<Value> {
    let mut calls: Vec<Value> = std::fs::read_dir(self.folder())
      .into_iter()
      .flatten()
      .filter_map(Result::ok)
      .filter_map(|entry| self.one(entry.file_name().to_str()?))
      .collect();

    calls.sort_by_key(|call| call["startedAt"].as_i64().unwrap_or(0));

    calls
  }

  /// One call as it stands: its request, its state, and its end where it has one.
  pub fn one(&self, id: &str) -> Option<Value> {
    if !readable(id) {
      return None;
    }

    let place = self.place(id);
    let request: Kept = read(&place.join(REQUEST))?;
    let end: Option<End> = read(&place.join(END));
    let state = match &end {
      Some(end) => end.state.clone(),
      None if self.running.lock().expect("the running calls are held").contains(id) => "running".to_owned(),
      None => "lost".to_owned(),
    };
    let mut answer = serde_json::to_value(&request).ok()?;

    answer["state"] = json!(state);

    if let Some(end) = end {
      answer["endedAt"] = json!(end.ended_at);
      answer["summary"] = json!(end.summary);
      answer["wake"] = end.wake;
    }

    Some(answer)
  }

  // The agent woken: the receiver is asked to queue a fact naming the call in the caller's room.
  async fn awake(&self, request: &Kept, state: &str, summary: &str) -> Result<i64, String> {
    let address = published("RECEIVER_ADDRESS", &self.data_root, "env/receiver.env")
      .ok_or("no receiver address: RECEIVER_ADDRESS is set nowhere")?;
    let token = published("RECEIVER_TOKEN", &self.data_root, "env/receiver.env")
      .ok_or("no receiver token: RECEIVER_TOKEN is set nowhere")?;
    let body = fact(request, state, summary);

    post_json(&address, "/instructions", &token, &body).await
  }
}

/// A fresh id for a call of `tool`: its namespace and name joined by `-`, then eight hexadecimal digits.
fn new_id(tool: &str) -> String {
  let named: String = tool
    .chars()
    .map(|character| if well_formed(character) { character } else { '-' })
    .collect();
  let nonce = RandomState::new().hash_one(SystemTime::now()) as u32;

  format!("{named}-{nonce:08x}")
}

// Whether `id` can name a call: letters, digits, `_` and `-` alone.
fn readable(id: &str) -> bool {
  !id.is_empty() && id.chars().all(well_formed)
}

fn well_formed(character: char) -> bool {
  character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

/// Now, in epoch milliseconds.
fn now_ms() -> i64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as i64)
}

// The line a wake carries for an answer: the tool and the first line of the first text it answered.
fn answered_summary(tool: &str, result: &Value) -> String {
  let first = result["content"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|piece| piece["text"].as_str())
    .next()
    .unwrap_or_default();

  line(&format!("{tool} answered: {}", first_line(first)))
}

fn first_line(text: &str) -> &str {
  text.lines().next().unwrap_or_default()
}

fn line(text: &str) -> String {
  text.chars().take(SUMMARY_CHARS).collect()
}

// The fact a call's end is queued as: the call named, its state, its summary, and what the caller asked to be
// handed back. The answer itself is read through the tool host's own route, `use/<id>`.
fn fact(request: &Kept, state: &str, summary: &str) -> Value {
  json!({
    "agentName": request.agent,
    "room": request.room,
    "said": {
      "fact": {
        "use": request.id,
        "tool": request.tool,
        "state": state,
        "summary": summary,
        "memory": request.memory,
      },
      "order": null,
    },
    "sort": SORT,
    "summary": summary,
    "kind": null,
    "expiresAt": null,
    "events": [],
    "ridesUntil": null,
    "via": request.via,
    "run": null,
  })
}

// A variable of the environment, else the value the env file `file` under `root` gives it.
fn published(name: &str, root: &Path, file: &str) -> Option<String> {
  std::env::var(name).ok().filter(|value| !value.is_empty()).or_else(|| {
    dotenvy::from_path_iter(root.join(file))
      .ok()?
      .filter_map(Result::ok)
      .find(|(key, _)| key == name)
      .map(|(_, value)| value)
      .filter(|value| !value.is_empty())
  })
}

// One JSON body posted to the receiver, answering the instruction's id.
async fn post_json(address: &str, path: &str, token: &str, body: &Value) -> Result<i64, String> {
  let base: Uri = address
    .parse()
    .map_err(|why| format!("the receiver address `{address}` does not read: {why}"))?;
  let host = base
    .host()
    .ok_or_else(|| format!("the receiver address `{address}` names no host"))?
    .to_owned();
  let port = base.port_u16().unwrap_or(80);
  let stream = TcpStream::connect((host.as_str(), port))
    .await
    .map_err(|why| format!("the receiver at {address} is out of reach: {why}"))?;
  let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
    .await
    .map_err(|why| format!("the receiver at {address} could not be spoken to: {why}"))?;

  tokio::spawn(connection);

  let written = serde_json::to_vec(body).map_err(|why| why.to_string())?;
  let request = Request::builder()
    .method("POST")
    .uri(path)
    .header("host", format!("{host}:{port}"))
    .header("authorization", format!("Bearer {token}"))
    .header("content-type", "application/json")
    .body(Full::new(Bytes::from(written)))
    .map_err(|why| format!("the wake could not be built: {why}"))?;
  let answer = sender
    .send_request(request)
    .await
    .map_err(|why| format!("the receiver at {address} is out of reach: {why}"))?;
  let status = answer.status();
  let body = answer
    .into_body()
    .collect()
    .await
    .map_err(|why| format!("the receiver at {address} answered unreadably: {why}"))?
    .to_bytes();
  let said: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);

  if status != StatusCode::OK && status != StatusCode::CREATED {
    return Err(format!(
      "the receiver answered {status}: {}: {}",
      said["code"].as_str().unwrap_or("refused"),
      said["error"].as_str().unwrap_or_default()
    ));
  }

  said["id"]
    .as_i64()
    .ok_or_else(|| format!("the receiver answered {status} with no instruction id"))
}

/// `value` kept as JSON at `path`, readable by the host's user alone, written whole or not at all.
fn keep(path: &Path, value: &impl Serialize) -> Result<(), HostError> {
  use std::io::Write as _;
  use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};

  if let Some(folder) = path.parent() {
    std::fs::DirBuilder::new()
      .recursive(true)
      .mode(0o700)
      .create(folder)
      .map_err(|why| HostError::Unwritable(format!("{}: {why}", folder.display())))?;
  }

  let text = serde_json::to_string_pretty(value).map_err(|why| HostError::Unwritable(why.to_string()))?;
  let written = path.with_extension("json.part");

  std::fs::OpenOptions::new()
    .write(true)
    .create(true)
    .truncate(true)
    .mode(0o600)
    .open(&written)
    .and_then(|mut file| file.write_all(format!("{text}\n").as_bytes()))
    .and_then(|()| std::fs::rename(&written, path))
    .map_err(|why| HostError::Unwritable(format!("{}: {why}", path.display())))
}

// The JSON kept at `path`, where it stands and reads.
fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
  serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

// The records a page of calls answers with, in the order the calls were asked.
pub fn page(calls: Vec<Value>) -> Value {
  json!({ "items": calls })
}
