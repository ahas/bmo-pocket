//! How a placed tool is run (docs/mcp-host.md, 「A call」 and 「Shared helpers」): a process per call handed its
//! call on standard input, or one MCP server per record; each in a clean environment, with a call-back socket of
//! its own into the host and the sockets of the shared helpers it names, which a record keeps until it goes.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;

use bmo_tool::manifest::Mode;
use bmo_tool::protocol::{Asked, CallBack, Handed, START_VARIABLE};
use bmo_tool::{Answer, Kept, Reach, Session};
use rmcp::model::CallToolRequestParams;
use rmcp::service::{Peer, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::{JoinHandle, JoinSet};

use crate::caller::record_gone;
use crate::drain::InHand;
use crate::handler::{Host, unknown};
use crate::placed::PlacedTool;
use crate::records::now_ms;

// How many lines of what a failed call wrote on standard error its answer carries.
const SAID_LINES: usize = 5;

/// A tool result in the protocol's shape, saying the call failed and why.
pub fn failed(why: impl std::fmt::Display) -> Value {
  serde_json::to_value(Answer::failure(why)).unwrap_or(Value::Null)
}

// A fresh name, for a socket no other call holds.
fn fresh(prefix: &str) -> String {
  let bytes: [u8; 8] = rand::random();

  format!("{prefix}-{}.sock", bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>())
}

/// The variables of the host's own environment every call is handed, where the host holds them.
pub const KEPT: [&str; 3] = ["PATH", "HOME", "XDG_RUNTIME_DIR"];

/// The command a placed tool's run words name: started in its placed folder, the first word read there where a
/// file of that name stands in it, in a clean environment, each layer laid over the one before it — the host's
/// path, home and runtime directory and each variable the host is told to pass on, where it holds them; the
/// record's variables; and those its variables file holds, read afresh — its own process group, ended with the
/// host.
fn command(host: &Host, tool: &PlacedTool, session: &Session) -> Command {
  let run = &tool.manifest.tool.run;
  let own = tool.dir.join(&run[0]);
  let program = if own.is_file() { own.into_os_string() } else { run[0].clone().into() };
  let mut command = Command::new(program);

  command.args(&run[1..]).current_dir(&tool.dir).env_clear();

  for kept in KEPT.iter().copied().chain(host.pass_env.iter().map(String::as_str)) {
    if let Some(value) = std::env::var_os(kept) {
      command.env(kept, value);
    }
  }

  command.envs(&session.variables);

  match session.file_variables() {
    Ok(variables) => {
      command.envs(variables);
    }
    Err(why) => eprintln!("mcp-host: {} adds nothing: {why}", tool.folder),
  }

  ended_with_host(&mut command);
  command
}

/// A command started in a process group of its own, ended when the host ends.
pub fn ended_with_host(command: &mut Command) {
  command.process_group(0).kill_on_drop(true);

  // SAFETY: the closure runs in the child between fork and exec and calls `prctl` alone, which is
  // async-signal-safe; it asks the kernel to end the child when the host ends.
  unsafe {
    command.pre_exec(|| {
      libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
      Ok(())
    });
  }
}

// The process group of one call, ended when it is dropped while still held.
struct Group(Option<u32>);

impl Drop for Group {
  fn drop(&mut self) {
    end_group(self.0);
  }
}

// Every process of a group ended.
fn end_group(pid: Option<u32>) {
  if let Some(pid) = pid.and_then(|pid| i32::try_from(pid).ok()) {
    // SAFETY: `killpg` only sends a signal; a group already gone is answered with an error, ignored here.
    unsafe {
      libc::killpg(pid, libc::SIGKILL);
    }
  }
}

/// A call's own socket into the host, taken away as it is dropped.
pub struct CallBackSocket {
  pub path: PathBuf,
  task: JoinHandle<()>,
}

impl Drop for CallBackSocket {
  fn drop(&mut self) {
    self.task.abort();
    let _ = std::fs::remove_file(&self.path);
  }
}

/// A call-back socket bound for `session`: each connection asks one thing on one line and is answered on one,
/// under the same record and through the same checks as the session's own calls.
pub fn call_back(host: &Arc<Host>, session: &Session) -> std::io::Result<CallBackSocket> {
  std::fs::create_dir_all(&host.placed.sockets)?;

  let path = host.placed.sockets.join(fresh("call"));
  let listener = UnixListener::bind(&path)?;
  let host = host.clone();
  let session = Arc::new(session.clone());
  // Each connection's asking is held in the socket's own task, so the socket taken away as its call ends takes
  // every call made through it along.
  let task = tokio::spawn(async move {
    let mut asking = JoinSet::new();

    loop {
      tokio::select! {
        accepted = listener.accept() => {
          let Ok((stream, _)) = accepted else {
            break;
          };

          asking.spawn(answer_connection(host.clone(), session.clone(), stream));
        }
        Some(_) = asking.join_next(), if !asking.is_empty() => {}
      }
    }

    while asking.join_next().await.is_some() {}
  });

  Ok(CallBackSocket { path, task })
}

// One connection to a call-back socket: one thing asked on one line, answered on one.
async fn answer_connection(host: Arc<Host>, session: Arc<Session>, stream: UnixStream) {
  let (reading, mut writing) = stream.into_split();
  let mut line = String::new();

  if BufReader::new(reading).read_line(&mut line).await.is_err() {
    return;
  }

  let said = match serde_json::from_str::<CallBack>(&line) {
    Ok(asked) => answer_call_back(&host, &session, asked).await,
    Err(why) => failed(format!("the call-back socket was asked what it does not read: {why}")),
  };
  let _ = writing.write_all(format!("{said}\n").as_bytes()).await;
}

// What a call-back socket answers: a call by full name, or which entry claims an address.
fn answer_call_back<'a>(
  host: &'a Arc<Host>,
  session: &'a Session,
  asked: CallBack,
) -> Pin<Box<dyn Future<Output = Value> + Send + 'a>> {
  Box::pin(async move {
    match asked {
      CallBack::Call { entry, arguments } => match host.find_full(session, &entry, true) {
        Some(found) => {
          host.answer(&found, session, arguments).await.unwrap_or_else(|why| failed(why.message))
        }
        None => failed(unknown(&entry)),
      },
      CallBack::Claim { address } => host.claim(session, &address).await,
    }
  })
}

/// What a call whose caller is gone fails with (docs/mcp-host.md, 「The host's stop」).
pub const CALLER_GONE: &str = "it was cancelled: its caller is gone";

/// One placed entry asked one thing under a record, answered as the tool answered or with why it did not: a
/// call through a process of its own or the record's server, each with what it needs started first. It ends with
/// its record, and a call of an entry that does not hold the host's stop ends at the stop.
pub async fn ask(
  host: &Arc<Host>,
  tool: &Arc<PlacedTool>,
  session: &Session,
  asked: Asked,
) -> Result<Value, String> {
  let held = (!ends_at_stop(tool, &asked)).then(|| host.drain.call(in_hand(tool, session, &asked)));
  // Dropping the asking ends its process group, so neither ending leaves a process of the call behind.
  let answered = tokio::select! {
    answer = asking(host, tool, session, asked) => answer,
    () = record_gone(session) => Err(CALLER_GONE.to_owned()),
    () = host.drain.stopping(), if held.is_none() => Err("the host stopped while it ran".to_owned()),
  };

  drop(held);
  answered
}

// A call in hand as the host names it: the entry it asks, or the tool for a resource listing or read.
fn in_hand(tool: &PlacedTool, session: &Session, asked: &Asked) -> InHand {
  let entry = match asked {
    Asked::Call { entry, .. } | Asked::Claim { entry, .. } => entry.clone(),
    _ => tool.manifest.tool.name.clone(),
  };

  InHand {
    entry,
    holder: session.holder_name.clone(),
    started_at: now_ms(),
  }
}

// Whether what is asked is a call of an entry declaring that it does not hold the host's stop.
fn ends_at_stop(tool: &PlacedTool, asked: &Asked) -> bool {
  let Asked::Call { entry, .. } = asked else {
    return false;
  };

  tool
    .manifest
    .entries
    .iter()
    .any(|declared| declared.ends_at_stop && declared.full_name() == *entry)
}

async fn asking(
  host: &Arc<Host>,
  tool: &Arc<PlacedTool>,
  session: &Session,
  asked: Asked,
) -> Result<Value, String> {
  // A personal tool is relayed to its owner's runner (docs/mcp-host.md, 「Personal tools」).
  if let Some(owner) = &tool.owner {
    let result = relayed(host, tool, owner, session, asked).await;

    host.placed.sweep();
    return result;
  }

  // An external server's tool is asked for its calls alone, on its registration's connection.
  if let Some(external) = &tool.external {
    return match asked {
      Asked::Call { entry, arguments } => external.call(&entry, arguments).await,
      _ => Err("an external server's tool is asked for its calls alone".to_owned()),
    };
  }

  let keeping = session.kept.get_or_keep(Placements::default);
  // A claim reads the address alone, so no helper is started for it.
  let shared = match asked {
    Asked::Claim { .. } => BTreeMap::new(),
    _ => keeping.helpers(host, tool, session).await?,
  };

  if let (Mode::Server, Asked::Call { entry, arguments }) = (tool.manifest.tool.mode, &asked) {
    let name = entry.rsplit('/').next().unwrap_or(entry).to_owned();

    return keeping.served(host, tool, session, shared, name, arguments.clone()).await;
  }

  let callback =
    call_back(host, session).map_err(|why| format!("its call-back socket could not be bound: {why}"))?;
  let mut handed = Handed::new(asked, session.clone());

  handed.callback = Some(callback.path.clone());
  handed.shared = shared;

  let result = once(host, tool, session, &handed).await;

  drop(callback);
  // A version a take or an upgrade retired while this call ran is taken away once nothing else holds it.
  host.placed.sweep();
  result
}

// One call of a personal tool relayed to its owner's runner: the host's relay started in the tool's placed folder at
// home, in a clean environment of the host's path, home and runtime directory alone, and handed the call on standard
// input as any tool's executable is. The tool is handed its record without its variables, and with no call-back
// socket or helper, since it runs on its owner's machine; the relay is told the owner, the tool, its version, its
// run words, the runner the record names, and what its manifest's `[permissions]` names where the record's reach
// lets the tool have it, the call being refused where it does not.
async fn relayed(
  host: &Host,
  tool: &PlacedTool,
  owner: &str,
  session: &Session,
  asked: Asked,
) -> Result<Value, String> {
  let Asked::Call { .. } = asked else {
    return Err("a personal tool is asked for its calls alone".to_owned());
  };
  let Some(runner) = &session.runner else {
    return Err("the session's record names no runner its owner's personal tools run on".to_owned());
  };
  let Some((program, words)) = host.placed.relay.split_first() else {
    return Err("the host names no relay".to_owned());
  };
  let permissions = &tool.manifest.permissions;

  if permissions.asks() {
    let reach = session.personal_reach().unwrap_or_else(|why| {
      eprintln!(
        "mcp-host: the record held by {} reaches neither the network nor a secret: {why}",
        session.holder_name
      );
      Reach::default()
    });

    for (asks, reaches, what) in [
      (!permissions.network.is_empty(), reach.network, "the network"),
      (!permissions.secrets.is_empty(), reach.secrets, "a secret"),
    ] {
      if asks && !reaches {
        return Err(format!(
          "tools/{} asks for {what}, which the organisation does not allow this person's personal apps",
          tool.folder
        ));
      }
    }
  }

  let secret_space = match (&session.secret_space, permissions.secrets.is_empty()) {
    (_, true) => None,
    (Some(space), false) => Some(space.clone()),
    (None, false) => {
      return Err(format!(
        "tools/{} names secrets, and the session's record names no secret space to hand them from",
        tool.folder
      ));
    }
  };
  let request = crate::relay::Request {
    owner: owner.to_owned(),
    tool: tool.folder.clone(),
    version: tool.version(),
    run: tool.manifest.tool.run.clone(),
    runner: runner.clone(),
    network: permissions.network.clone(),
    secrets: permissions.secrets.clone(),
    secret_space,
  };
  let request =
    serde_json::to_string(&request).map_err(|why| format!("the relay could not be asked: {why}"))?;
  let record = Session {
    variables: BTreeMap::new(),
    variables_file: None,
    ..session.clone()
  };
  let handed = Handed::new(asked, record);
  let mut command = Command::new(program);

  command.args(words).arg(request).current_dir(&tool.dir).env_clear();

  for kept in KEPT {
    if let Some(value) = std::env::var_os(kept) {
      command.env(kept, value);
    }
  }

  ended_with_host(&mut command);
  run_once(command, tool, &handed).await
}

// One process run to its end for one thing asked, its answer read off its standard output.
async fn once(host: &Host, tool: &PlacedTool, session: &Session, handed: &Handed) -> Result<Value, String> {
  run_once(command(host, tool, session), tool, handed).await
}

// One process of a command run to its end for one thing asked, its answer read off its standard output.
async fn run_once(mut command: Command, tool: &PlacedTool, handed: &Handed) -> Result<Value, String> {
  let input = serde_json::to_vec(handed).map_err(|why| format!("the call could not be written: {why}"))?;
  let mut child = command
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|why| format!("tools/{} could not be started: {why}", tool.folder))?;
  // Ended with its whole group unless it ends of itself: a call whose caller goes away and drops the asking
  // leaves nothing it started behind.
  let mut group = Group(child.id());

  // Written beside the reading of its answer, so a call that answers before it has read everything never
  // waits on a full pipe.
  if let Some(mut stdin) = child.stdin.take() {
    tokio::spawn(async move {
      let _ = stdin.write_all(&input).await;
    });
  }

  let done = match child.wait_with_output().await {
    Ok(done) => done,
    Err(why) => return Err(format!("its process could not be read: {why}")),
  };
  group.0 = None;
  let stderr = String::from_utf8_lossy(&done.stderr);

  for line in stderr.lines() {
    eprintln!("mcp-host: tools/{}: {line}", tool.folder);
  }

  let said: Vec<&str> = stderr
    .trim()
    .lines()
    .rev()
    .take(SAID_LINES)
    .collect::<Vec<_>>()
    .into_iter()
    .rev()
    .collect();
  let ended = || {
    let code = done.status.code().map_or_else(|| "a signal".to_owned(), |code| format!("code {code}"));

    if said.is_empty() {
      format!("it ended with {code}")
    } else {
      format!("it ended with {code}: {}", said.join("\n"))
    }
  };

  if !done.status.success() {
    return Err(ended());
  }

  serde_json::from_slice(&done.stdout).map_err(|why| format!("its answer does not read: {why}"))
}

// A record's server of one placed tool, holding the version it started from for as long as it runs.
struct Server {
  peer: Peer<RoleClient>,
  running: std::sync::Mutex<Option<RunningService<RoleClient, ()>>>,
  _callback: CallBackSocket,
  _tool: Arc<PlacedTool>,
}

impl Server {
  async fn end(&self) {
    let running = self.running.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();

    if let Some(running) = running {
      let _ = running.cancel().await;
    }
  }
}

// A record's running shared helper, holding the version it started from for as long as it runs.
struct Helper {
  child: Child,
  socket: PathBuf,
  _tool: Arc<PlacedTool>,
}

impl Helper {
  // Asked to end, and waited for until it has ended, so it ends what it holds outside its own group — the
  // branches it opened give their pool places back — then whatever it left standing in its group ended, and
  // its socket taken away.
  async fn end(mut self) {
    if let Some(pid) = self.child.id().and_then(|pid| i32::try_from(pid).ok()) {
      // SAFETY: a signal to the helper this record started, which has not been waited for, so its pid is its own.
      unsafe {
        libc::kill(pid, libc::SIGTERM);
      }

      let _ = self.child.wait().await;
      end_group(Some(pid as u32));
    }

    let _ = self.child.start_kill();
    let _ = std::fs::remove_file(&self.socket);
  }
}

/// What a record keeps of its placed tools: each server tool's server and each shared helper, ended with it.
#[derive(Default)]
pub struct Placements {
  servers: Mutex<BTreeMap<String, Arc<Server>>>,
  helpers: Mutex<BTreeMap<String, Helper>>,
}

impl Kept for Placements {
  fn close(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
    Box::pin(async move {
      for (_, server) in std::mem::take(&mut *self.servers.lock().await) {
        server.end().await;
      }

      for (_, helper) in std::mem::take(&mut *self.helpers.lock().await) {
        helper.end().await;
      }
    })
  }
}

// Waits for a helper to say on its standard output that it listens, or to end without saying so.
async fn listening(child: &mut Child) -> Result<(), String> {
  let Some(stdout) = child.stdout.take() else {
    return Err("its output could not be read".to_owned());
  };
  let mut lines = BufReader::new(stdout).lines();

  match lines.next_line().await {
    Ok(Some(_)) => {
      // What it says after it listens goes to the log, so a full pipe never stops it.
      tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
          eprintln!("mcp-host: helper: {line}")
        }
      });
      Ok(())
    }
    _ => Err("it ended before it listened".to_owned()),
  }
}

impl Placements {
  /// The socket of each shared helper the tool names, each started for this record where it is not yet.
  async fn helpers(
    &self,
    host: &Arc<Host>,
    tool: &PlacedTool,
    session: &Session,
  ) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut sockets = BTreeMap::new();

    if tool.manifest.tool.shared.is_empty() {
      return Ok(sockets);
    }

    let mut helpers = self.helpers.lock().await;

    for name in &tool.manifest.tool.shared {
      if let Some(running) = helpers.get_mut(name) {
        if running.child.try_wait().is_ok_and(|ended| ended.is_none()) {
          sockets.insert(name.clone(), running.socket.clone());
          continue;
        }

        if let Some(gone) = helpers.remove(name) {
          gone.end().await;
        }
      }

      let catalogue = host.placed.current();
      let Some(helper) = catalogue.helpers.get(name) else {
        return Err(format!("no shared helper `{name}` is placed"));
      };
      let started = start_helper(host, helper, session)
        .await
        .map_err(|why| format!("the shared helper `{name}`: {why}"))?;

      sockets.insert(name.clone(), started.socket.clone());
      helpers.insert(name.clone(), started);
    }

    Ok(sockets)
  }

  /// One call of a server tool, on the record's server of it, started first where none stands; a call that
  /// fails ends the server, and the next call starts it again.
  #[allow(clippy::too_many_arguments)]
  async fn served(
    &self,
    host: &Arc<Host>,
    tool: &Arc<PlacedTool>,
    session: &Session,
    shared: BTreeMap<String, PathBuf>,
    name: String,
    arguments: Value,
  ) -> Result<Value, String> {
    let server = {
      let mut servers = self.servers.lock().await;

      match servers.get(&tool.folder) {
        Some(server) => server.clone(),
        None => {
          let started = Arc::new(start_server(host, tool, session, shared).await?);

          servers.insert(tool.folder.clone(), started.clone());
          started
        }
      }
    };
    let mut asked = CallToolRequestParams::new(name);

    if let Value::Object(arguments) = arguments {
      asked = asked.with_arguments(arguments);
    }

    let why = match server.peer.call_tool(asked).await {
      Ok(result) => return serde_json::to_value(result).map_err(|why| why.to_string()),
      Err(why) => format!("its server failed the call: {why}"),
    };

    if let Some(gone) = self.servers.lock().await.remove(&tool.folder) {
      gone.end().await;
    }

    Err(why)
  }
}

async fn start_helper(
  host: &Arc<Host>,
  helper: &Arc<PlacedTool>,
  session: &Session,
) -> Result<Helper, String> {
  std::fs::create_dir_all(&host.placed.sockets).map_err(|why| why.to_string())?;

  let socket = host.placed.sockets.join(fresh("helper"));
  let handed = Handed::new(
    Asked::Helper {
      socket: socket.clone(),
    },
    session.clone(),
  );
  let input = serde_json::to_vec(&handed).map_err(|why| why.to_string())?;
  let mut child = command(host, helper, session)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit())
    .spawn()
    .map_err(|why| format!("it could not be started: {why}"))?;

  if let Some(mut stdin) = child.stdin.take() {
    let _ = stdin.write_all(&input).await;
  }

  let mut started = Helper {
    child,
    socket,
    _tool: helper.clone(),
  };

  if let Err(why) = listening(&mut started.child).await {
    started.end().await;
    return Err(why);
  }

  Ok(started)
}

async fn start_server(
  host: &Arc<Host>,
  tool: &Arc<PlacedTool>,
  session: &Session,
  shared: BTreeMap<String, PathBuf>,
) -> Result<Server, String> {
  let callback =
    call_back(host, session).map_err(|why| format!("its call-back socket could not be bound: {why}"))?;
  let mut handed = Handed::new(Asked::Start, session.clone());

  handed.callback = Some(callback.path.clone());
  handed.shared = shared;

  let start = serde_json::to_string(&handed).map_err(|why| why.to_string())?;
  let mut command = command(host, tool, session);

  command.env(START_VARIABLE, start);

  let (process, _) = TokioChildProcess::builder(command)
    .stderr(Stdio::inherit())
    .spawn()
    .map_err(|why| format!("tools/{} could not be started: {why}", tool.folder))?;
  let running = ().serve(process).await.map_err(|why| format!("its server did not start: {why}"))?;

  Ok(Server {
    peer: running.peer().clone(),
    running: std::sync::Mutex::new(Some(running)),
    _callback: callback,
    _tool: tool.clone(),
  })
}

/// The arguments a claim's answer reads a page with, where it claims the page.
pub fn claimed(answer: &Value) -> Option<Value> {
  answer.get("arguments").filter(|arguments| !arguments.is_null()).cloned()
}

/// Where a call-back socket stands beside the control socket.
pub fn sockets_beside(control: &Path) -> PathBuf {
  control.parent().map_or_else(|| PathBuf::from("calls"), |dir| dir.join("calls"))
}

/// A claim answered: the entry and its arguments, or none.
pub fn claim_answer(found: Option<(String, Value)>) -> Value {
  match found {
    Some((entry, arguments)) => json!({ "entry": entry, "arguments": arguments }),
    None => json!({ "entry": null }),
  }
}
