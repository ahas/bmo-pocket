//! How a placed tool is run (docs/mcp-host.md, 「A call」 and 「Shared helpers」): a process per call handed its
//! call on standard input, or one MCP server per record; each in a clean environment, with a call-back socket of
//! its own into the host and the sockets of the shared helpers it names, which a record keeps until it goes.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use bmo_tool::manifest::{Entry as Declared, Mode};
use bmo_tool::protocol::{Asked, CallBack, Handed, START_VARIABLE};
use bmo_tool::{Answer, Kept, Session};
use rmcp::model::CallToolRequestParams;
use rmcp::service::{Peer, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::handler::{Host, unknown};
use crate::placed::PlacedTool;

// How many lines of what a failed call wrote on standard error its answer carries.
const SAID_LINES: usize = 5;

// How long a helper asked to end is given before its group is ended.
const HELPER_GRACE: Duration = Duration::from_secs(10);

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
  let task = tokio::spawn(async move {
    while let Ok((stream, _)) = listener.accept().await {
      let host = host.clone();
      let session = session.clone();

      tokio::spawn(async move {
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
      });
    }
  });

  Ok(CallBackSocket { path, task })
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

/// How long one call may take: the entry's timeout, else the tool's, or none for an entry declared unbounded,
/// whose call waits on what it set off for as long as that takes.
#[derive(Clone, Copy)]
pub struct Bound(pub Option<Duration>);

impl Bound {
  pub fn of(declared: Option<&Declared>, tool_seconds: u64) -> Self {
    if declared.is_some_and(|entry| entry.unbounded) {
      return Self(None);
    }

    let seconds = declared.and_then(|entry| entry.timeout).unwrap_or(tool_seconds);

    Self(Some(Duration::from_secs(seconds)))
  }

  /// The work run to its end within the bound, or that it ran past it.
  pub async fn on<T>(self, work: impl Future<Output = T>) -> Result<T, String> {
    match self.0 {
      Some(bound) => tokio::time::timeout(bound, work)
        .await
        .map_err(|_| format!("it did not answer within {} seconds", bound.as_secs())),
      None => Ok(work.await),
    }
  }
}

/// One placed entry asked one thing under a record, answered as the tool answered or with why it did not: a
/// call through a process of its own or the record's server, each with what it needs started first.
pub async fn ask(
  host: &Arc<Host>,
  tool: &Arc<PlacedTool>,
  declared: Option<&Declared>,
  session: &Session,
  asked: Asked,
) -> Result<Value, String> {
  let bound = Bound::of(declared, tool.manifest.tool.timeout);

  // An external server's tool is asked for its calls alone, on its registration's connection.
  if let Some(external) = &tool.external {
    return match asked {
      Asked::Call { entry, arguments } => external.call(&entry, arguments, bound).await,
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

    return keeping.served(host, tool, session, shared, name, arguments.clone(), bound).await;
  }

  let callback =
    call_back(host, session).map_err(|why| format!("its call-back socket could not be bound: {why}"))?;
  let mut handed = Handed::new(asked, session.clone());

  handed.callback = Some(callback.path.clone());
  handed.shared = shared;

  let result = once(host, tool, session, &handed, bound).await;

  drop(callback);
  // A version a take or an upgrade retired while this call ran is taken away once nothing else holds it.
  host.placed.sweep();
  result
}

// One process run to its end for one thing asked, its answer read off its standard output.
async fn once(
  host: &Host,
  tool: &PlacedTool,
  session: &Session,
  handed: &Handed,
  bound: Bound,
) -> Result<Value, String> {
  let input = serde_json::to_vec(handed).map_err(|why| format!("the call could not be written: {why}"))?;
  let mut child = command(host, tool, session)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|why| format!("tools/{} could not be started: {why}", tool.folder))?;
  // Ended with its whole group unless it ends of itself: a call that runs past its bound, or whose caller
  // goes away and drops the asking, leaves nothing it started behind.
  let mut group = Group(child.id());

  // Written beside the reading of its answer, so a call that answers before it has read everything never
  // waits on a full pipe.
  if let Some(mut stdin) = child.stdin.take() {
    tokio::spawn(async move {
      let _ = stdin.write_all(&input).await;
    });
  }

  let done = match bound.on(child.wait_with_output()).await? {
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
  // Asked to end first, so it ends what it holds outside its own group — the branches it opened give their
  // pool places back — then its group ended, whatever still stands of it, and its socket taken away.
  async fn end(mut self) {
    if let Some(pid) = self.child.id().and_then(|pid| i32::try_from(pid).ok()) {
      // SAFETY: a signal to the helper this record started, which has not been waited for, so its pid is its own.
      unsafe {
        libc::kill(pid, libc::SIGTERM);
      }

      let _ = tokio::time::timeout(HELPER_GRACE, self.child.wait()).await;
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

// Waits, within `bound`, for a helper to say on its standard output that it listens.
async fn listening(child: &mut Child, bound: Duration) -> Result<(), String> {
  let Some(stdout) = child.stdout.take() else {
    return Err("its output could not be read".to_owned());
  };
  let mut lines = BufReader::new(stdout).lines();

  match tokio::time::timeout(bound, lines.next_line()).await {
    Ok(Ok(Some(_))) => {
      // What it says after it listens goes to the log, so a full pipe never stops it.
      tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
          eprintln!("mcp-host: helper: {line}")
        }
      });
      Ok(())
    }
    Ok(_) => Err("it ended before it listened".to_owned()),
    Err(_) => Err(format!("it did not listen within {} seconds", bound.as_secs())),
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
  /// fails or outlasts its bound ends the server, and the next call starts it again.
  #[allow(clippy::too_many_arguments)]
  async fn served(
    &self,
    host: &Arc<Host>,
    tool: &Arc<PlacedTool>,
    session: &Session,
    shared: BTreeMap<String, PathBuf>,
    name: String,
    arguments: Value,
    bound: Bound,
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

    let why = match bound.on(server.peer.call_tool(asked)).await {
      Ok(Ok(result)) => return serde_json::to_value(result).map_err(|why| why.to_string()),
      Ok(Err(why)) => format!("its server failed the call: {why}"),
      Err(why) => why,
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

  if let Err(why) = listening(&mut started.child, Duration::from_secs(helper.manifest.tool.timeout)).await {
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
  let bound = Duration::from_secs(tool.manifest.tool.timeout);
  let running = match tokio::time::timeout(bound, ().serve(process)).await {
    Ok(Ok(running)) => running,
    Ok(Err(why)) => return Err(format!("its server did not start: {why}")),
    Err(_) => return Err(format!("its server did not start within {} seconds", bound.as_secs())),
  };

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
