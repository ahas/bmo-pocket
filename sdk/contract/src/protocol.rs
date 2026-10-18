//! What the host and a placed tool say to each other (docs/mcp-host.md, 「A call」): what a call's process
//! is handed on standard input, what a server is handed in its environment, and what is asked and
//! answered on a call-back socket. A Rust tool's `main` answers its call with [`answer_standard_io`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::answer::{Answer, CallError};
use crate::kept::Kept;
use crate::manifest::PROTOCOL;
use crate::resources::Resources;
use crate::session::Session;
use crate::tool::Served;

/// The variable a server tool is handed its start in.
pub const START_VARIABLE: &str = "BMO_TOOL_START";

/// What a placed executable is asked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Asked {
  /// One entry called with its arguments, answered with the protocol's tool result.
  Call { entry: String, arguments: Value },
  /// Whether the entry reads this address, answered `{"arguments": …}` or `{"arguments": null}`.
  Claim { entry: String, address: String },
  /// The resources it serves, answered `{"resources": [...]}`.
  ResourcesList,
  /// One resource, answered `{"contents": [...]}`, or `{"error": "<why>"}` where it holds none.
  ResourcesRead { uri: String },
  /// A shared helper started for a record, which listens on the socket it is handed.
  Helper { socket: PathBuf },
  /// A server's start, handed in [`START_VARIABLE`] rather than on standard input.
  Start,
}

/// Everything a placed executable is handed: what it is asked, under which record, and the sockets it
/// reaches the host and its shared helpers by.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Handed {
  pub protocol: u32,
  #[serde(flatten)]
  pub asked: Asked,
  pub session: Session,
  /// The call's own socket back into the host; none for a helper.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub callback: Option<PathBuf>,
  /// Each shared helper's socket, by the helper's name.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub shared: BTreeMap<String, PathBuf>,
}

impl Handed {
  pub fn new(asked: Asked, session: Session) -> Self {
    Self {
      protocol: PROTOCOL,
      asked,
      session,
      callback: None,
      shared: BTreeMap::new(),
    }
  }
}

/// What a tool asks on its call-back socket, one JSON object on one line, answered the same way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallBack {
  /// An entry called under the same record, by its full name; answered with its tool result.
  Call {
    entry: String,
    #[serde(default)]
    arguments: Value,
  },
  /// Which entry the record holds claims this address, answered `{"entry": …, "arguments": …}` or
  /// `{"entry": null}`.
  Claim { address: String },
}

/// The sockets a call was handed: its call-back socket into the host and each shared helper's, kept under
/// the call's record so the tool it calls reaches them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sockets {
  pub callback: Option<PathBuf>,
  pub shared: BTreeMap<String, PathBuf>,
}

impl Kept for Sockets {}

impl Sockets {
  /// The sockets the call under this record was handed; none where it was handed none.
  pub fn of(session: &Session) -> Arc<Self> {
    session.kept.get_or_keep(Self::default)
  }

  /// One thing asked on the call's call-back socket, answered as the host answers it.
  pub async fn call_back(&self, asked: &CallBack) -> Result<Value, String> {
    let socket = self.callback.as_deref().ok_or("this call was handed no call-back socket")?;

    ask_line(socket, asked).await
  }

  /// The socket of the shared helper of this name.
  pub fn helper(&self, name: &str) -> Result<&Path, String> {
    self
      .shared
      .get(name)
      .map(PathBuf::as_path)
      .ok_or_else(|| format!("this call was handed no socket of the shared helper `{name}`"))
  }
}

/// One JSON object asked on one line of a socket, and the one line it is answered with read as `R`: how a call
/// asks its call-back socket and its shared helpers.
pub async fn ask_line<R: DeserializeOwned>(socket: &Path, asked: &impl Serialize) -> Result<R, String> {
  let at = socket.display();
  let stream = UnixStream::connect(socket)
    .await
    .map_err(|why| format!("{at} could not be reached: {why}"))?;
  let (reading, mut writing) = stream.into_split();
  let mut line =
    serde_json::to_string(asked).map_err(|why| format!("what was asked could not be written: {why}"))?;

  line.push('\n');
  writing
    .write_all(line.as_bytes())
    .await
    .map_err(|why| format!("{at} could not be asked: {why}"))?;

  let mut answer = String::new();

  BufReader::new(reading)
    .read_line(&mut answer)
    .await
    .map_err(|why| format!("{at} could not be read: {why}"))?;

  if answer.trim().is_empty() {
    return Err(format!("{at} answered nothing"));
  }

  serde_json::from_str(&answer).map_err(|why| format!("the answer of {at} does not read: {why}"))
}

/// One call answered by the Rust tools of an executable: what `handed` asks, called on the tool of that
/// full name with the sockets it was handed kept under its record, and the answer the executable writes on
/// standard output.
pub async fn answer_call(tools: &[&dyn Served], handed: Handed) -> Result<Answer, CallError> {
  let Handed {
    asked,
    session,
    callback,
    shared,
    ..
  } = handed;
  let Asked::Call { entry, arguments } = asked else {
    return Ok(Answer::failure("this executable answers calls alone"));
  };
  let Some(tool) = tools.iter().find(|tool| format!("{}/{}", tool.namespace(), tool.name()) == entry)
  else {
    return Ok(Answer::failure(format!("this executable holds no entry {entry}")));
  };

  session.kept.get_or_keep(|| Sockets { callback, shared });
  tool.call(&session, arguments).await
}

/// A claim request answered: `{"arguments": …}`, the arguments the entry reads the address with, or null where
/// the executable holds no such entry or the entry does not claim the address.
pub fn answer_claim(tools: &[&dyn Served], entry: &str, address: &str) -> Value {
  let arguments = tools
    .iter()
    .find(|tool| format!("{}/{}", tool.namespace(), tool.name()) == entry)
    .and_then(|tool| tool.claim(address));

  serde_json::json!({ "arguments": arguments })
}

/// A resource request answered by the resources an executable serves: `{"resources": [...]}` for a listing,
/// and for a read the document's `{"contents": [...]}` or `{"error": "<why>"}`. An executable serving none
/// lists none and refuses every read.
pub async fn answer_resources(
  resources: Option<&dyn Resources>,
  session: &Session,
  asked: &Asked,
) -> Value {
  match (asked, resources) {
    (Asked::ResourcesList, Some(served)) => serde_json::json!({ "resources": served.list(session).await }),
    (Asked::ResourcesList, None) => serde_json::json!({ "resources": [] }),
    (Asked::ResourcesRead { uri }, Some(served)) => served
      .read(session, uri)
      .await
      .unwrap_or_else(|why| serde_json::json!({ "error": why })),
    (Asked::ResourcesRead { .. }, None) => {
      serde_json::json!({ "error": "this executable serves no resources" })
    }
    _ => serde_json::json!({ "error": "this is no resource request" }),
  }
}

/// One call, claim or resource request read from `input` and answered on `output` by the Rust tools of an
/// executable and the resources it serves, where it serves any: arguments that do not fit their tool are
/// answered as a failed call, and a request that does not read is refused with why.
pub async fn answer_read(
  tools: &[&dyn Served],
  resources: Option<&dyn Resources>,
  input: impl std::io::Read,
  output: impl std::io::Write,
) -> Result<(), String> {
  let handed: Handed =
    serde_json::from_reader(input).map_err(|why| format!("the call does not read: {why}"))?;
  let said = match &handed.asked {
    Asked::Claim { entry, address } => answer_claim(tools, entry, address),
    Asked::ResourcesList | Asked::ResourcesRead { .. } => {
      answer_resources(resources, &handed.session, &handed.asked).await
    }
    _ => serde_json::to_value(answer_call(tools, handed).await.unwrap_or_else(Answer::failure))
      .map_err(|why| format!("the answer could not be written: {why}"))?,
  };

  serde_json::to_writer(output, &said).map_err(|why| format!("the answer could not be written: {why}"))
}

/// A placed Rust tool's whole `main`: its call read off standard input and answered on standard output, a
/// call that does not read said on standard error and ended unclean, which the host takes as a failed call.
pub async fn answer_standard_io(tools: &[&dyn Served]) -> std::process::ExitCode {
  answer_standard_io_serving(tools, None).await
}

/// The whole `main` of a placed Rust tool whose manifest declares resources: its entries' calls and its
/// resource requests answered as [`answer_standard_io`] answers a call.
pub async fn answer_standard_io_serving(
  tools: &[&dyn Served],
  resources: Option<&dyn Resources>,
) -> std::process::ExitCode {
  match answer_read(tools, resources, std::io::stdin().lock(), std::io::stdout().lock()).await {
    Ok(()) => std::process::ExitCode::SUCCESS,
    Err(why) => {
      eprintln!("{why}");
      std::process::ExitCode::FAILURE
    }
  }
}
