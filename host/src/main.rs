//! The tool host's binary (docs/mcp-host.md). Its verbs:
//!
//! - `serve`: the host itself — every namespace on the loopback port, behind each session's bearer
//!   token, the control socket tool sessions are issued and revoked on, and, where a port is named for it,
//!   the console face a console reads the host's screen on.
//! - `call <namespace> <tool>`: one call of one placed tool as a session would make it, the
//!   session record as an issuer asks for it and the arguments read from standard input as
//!   `{"session": …, "arguments": …}` and the answer written to standard output as the protocol writes
//!   it. It is how a person or a test asks the tools without a session, and it reaches exactly what the
//!   host serves.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use bmo_tool::SessionRequest;
use clap::{Args, Parser, Subcommand};
use mcp_host::async_calls::{AsyncCalls, WakeTries};
use mcp_host::console::{Bundle, bundle_beside_binary};
use mcp_host::drain::Drain;
use mcp_host::handler::Host;
use mcp_host::placed::Placed;
use mcp_host::records::{Records, kept_token, now_ms};
use mcp_host::registry::Registry;
use mcp_host::running::sockets_beside;
use mcp_host::schedule::{self, Schedules};
use mcp_host::server::{self, Screen};
use serde::Deserialize;
use serde_json::Value;
use tokio::net::{TcpListener, UnixListener};
use tokio::signal::unix::{SignalKind, signal};

#[derive(Parser)]
#[command(
  name = "mcp-host",
  about = "The tool host: every session's MCP tools, served by bearer token."
)]
struct Cli {
  #[command(subcommand)]
  verb: Verb,
}

/// What a call reaches beyond the tool and its record: the variables of the host's own passed on.
#[derive(Args)]
struct Reach {
  /// A variable of the host's own environment every call is handed beside its own; given again for each.
  #[arg(long = "pass-env")]
  pass_env: Vec<String>,
}

#[derive(Subcommand)]
enum Verb {
  /// Serves every namespace on the loopback port, and the control socket.
  Serve {
    /// The loopback port sessions call on, which every line's configuration names; 0 takes any free one.
    #[arg(long, env = "BMO_MCP_HOST_PORT", default_value_t = 3290)]
    port: u16,
    /// The data root whose `tool-host/` directory holds the records.
    #[arg(long, env = "BMO_DATA_DIR")]
    data_root: Option<PathBuf>,
    /// The control socket; `bmo/mcp-host.sock` under the user's runtime directory by default.
    #[arg(long)]
    control: Option<PathBuf>,
    /// The file the issuer token is kept in, made where it does not stand; `tool-host/issuer-token` under the data
    /// root by default.
    #[arg(long)]
    issuer_token_file: Option<PathBuf>,
    /// How often lapsed records are dropped.
    #[arg(long, default_value_t = 60_000)]
    sweep_every_ms: u64,
    /// How often the schedules are read and those due called.
    #[arg(long, default_value_t = 1_000)]
    schedule_every_ms: u64,
    #[command(flatten)]
    reach: Reach,
    /// The placed tools' folder; `tools/` beside the `bin/` this binary stands in by default.
    #[arg(long)]
    tools: Option<PathBuf>,
    /// How long the changes under the tools' folder must have settled before it is read again.
    #[arg(long, default_value_t = 300)]
    settle_ms: u64,
    /// The first wait before a wake the receiver did not queue is asked again; each wait after it doubles, to a
    /// minute.
    #[arg(long, default_value_t = 1_000)]
    wake_retry_ms: u64,
    /// How long one try of a wake waits on the receiver before it counts as failed.
    #[arg(long, default_value_t = 30_000)]
    wake_try_ms: u64,
    #[command(flatten)]
    console: ConsoleFace,
  },
  /// Calls one tool as a session would: `{"session": …, "arguments": …}` on standard input.
  Call {
    /// The namespace the tool is shown under.
    namespace: String,
    /// The tool's name.
    tool: String,
    #[command(flatten)]
    reach: Reach,
    /// The data root whose `tool-host/registry/` names the registered tools.
    #[arg(long, env = "BMO_DATA_DIR")]
    data_root: Option<PathBuf>,
    /// The placed tools' folder; `tools/` beside the `bin/` this binary stands in by default.
    #[arg(long)]
    tools: Option<PathBuf>,
  },
}

/// The console face (docs/api.md, 「The console face」): opened only where a port is named for it.
#[derive(Args)]
struct ConsoleFace {
  /// The loopback port a console reads the host's screen on; none opens no console face, and 0 takes any free one.
  #[arg(long, env = "BMO_MCP_HOST_CONSOLE_PORT")]
  console_port: Option<u16>,
  /// The file the console token is kept in, made where it does not stand; `tool-host/console-token` under the data
  /// root by default.
  #[arg(long)]
  console_token_file: Option<PathBuf>,
  /// The folder the console bundle's build wrote into; `console-bundle/` beside the `bin/` this binary stands in by
  /// default.
  #[arg(long)]
  console_bundle: Option<PathBuf>,
}

// The console face as it is opened: its port, and what it serves behind the console token.
struct Opened {
  port: u16,
  screen: Screen,
}

impl ConsoleFace {
  // The face opened where a port is named, its token kept and its bundle read; why not where its token could not be
  // kept.
  fn open(self, data_root: &Path) -> Result<Option<Opened>, String> {
    let Some(port) = self.console_port else {
      return Ok(None);
    };
    let file = self.console_token_file.unwrap_or_else(|| data_root.join("tool-host/console-token"));
    let token = kept_token(&file)
      .map_err(|why| format!("the console token {} could not be kept: {why}", file.display()))?;
    let bundle = self
      .console_bundle
      .or_else(bundle_beside_binary)
      .and_then(|folder| Bundle::found(&folder));

    Ok(Some(Opened {
      port,
      screen: Screen { token, bundle },
    }))
  }
}

#[derive(Deserialize)]
struct Asked {
  session: SessionRequest,
  #[serde(default)]
  arguments: Value,
}

fn refused(why: impl std::fmt::Display) -> ExitCode {
  eprintln!("mcp-host: {why}");
  ExitCode::FAILURE
}

impl Reach {
  fn host(self, placed: Arc<Placed>) -> Host {
    Host {
      placed,
      pass_env: self.pass_env,
      drain: Drain::default(),
    }
  }
}

fn main() -> ExitCode {
  match Cli::parse().verb {
    Verb::Serve {
      port,
      data_root,
      control,
      issuer_token_file,
      sweep_every_ms,
      schedule_every_ms,
      reach,
      tools,
      settle_ms,
      wake_retry_ms,
      wake_try_ms,
      console,
    } => {
      let data_root = data_root.unwrap_or_else(default_data_root);
      let control = control.unwrap_or_else(|| runtime_dir().join("bmo/mcp-host.sock"));
      let issuer_file = issuer_token_file.unwrap_or_else(|| data_root.join("tool-host/issuer-token"));
      let issuer = match kept_token(&issuer_file) {
        Ok(token) => token,
        Err(why) => {
          return refused(format!("the issuer token {} could not be kept: {why}", issuer_file.display()));
        }
      };
      let console = match console.open(&data_root) {
        Ok(console) => console,
        Err(why) => return refused(why),
      };
      let placed = Arc::new(placed(tools, &data_root, &control));
      let host = Arc::new(reach.host(placed));

      match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(serve(
          (port, console),
          &data_root,
          (&control, issuer),
          Beats {
            sweep: Duration::from_millis(sweep_every_ms),
            schedule: Duration::from_millis(schedule_every_ms),
            settle: Duration::from_millis(settle_ms),
            wake: WakeTries {
              first_wait: Duration::from_millis(wake_retry_ms),
              span: Duration::from_millis(wake_try_ms),
            },
          },
          host,
        )),
        Err(why) => refused(why),
      }
    }
    Verb::Call {
      namespace,
      tool,
      reach,
      data_root,
      tools,
    } => {
      let data_root = data_root.unwrap_or_else(default_data_root);
      let placed = placed(tools, &data_root, &runtime_dir().join("bmo/mcp-host.sock"));

      placed.reread();
      call(&namespace, &tool, Arc::new(reach.host(Arc::new(placed))))
    }
  }
}

fn home() -> PathBuf {
  std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

fn default_data_root() -> PathBuf {
  home().join(".local/share/bmo/data")
}

// The user's runtime directory, where the sockets of this machine's services stand.
fn runtime_dir() -> PathBuf {
  std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

// The placed tools' catalogue: the folder named, or `tools/` beside the `bin/` this binary stands in, and the
// data root's registry beside it, its sockets beside the control socket.
fn placed(tools: Option<PathBuf>, data_root: &Path, control: &Path) -> Placed {
  let tools = tools
    .or_else(|| std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.parent()?.join("tools"))));

  Placed::new(tools, sockets_beside(control)).registered(Registry::of(data_root))
}

// The control socket, bound where only the host's own user reaches it.
fn bind_control(socket: &Path) -> io::Result<UnixListener> {
  if let Some(dir) = socket.parent() {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
  }

  match fs::remove_file(socket) {
    Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
    _ => {}
  }

  let listener = UnixListener::bind(socket)?;

  fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;

  Ok(listener)
}

// Ends once the host is asked to stop. The signals are taken as this is called, so one that comes from then on is
// the host's to answer.
fn stop_asked() -> impl Future<Output = ()> {
  let signals = (signal(SignalKind::terminate()), signal(SignalKind::interrupt()));

  async move {
    let (Ok(mut term), Ok(mut interrupt)) = signals else {
      return std::future::pending().await;
    };

    tokio::select! {
      _ = term.recv() => {}
      _ = interrupt.recv() => {}
    }
  }
}

// What the host keeps ended at its stop, once nothing is left that uses it: each record's servers and shared helpers,
// each waited on as a revocation waits, and each external server's connection.
async fn end_what_is_kept(records: &Records, host: &Host) {
  let mut ending = tokio::task::JoinSet::new();
  let lapsed = records.prune().into_iter().map(|(_, session)| session);

  for session in records.standing().into_iter().map(|(_, session)| session).chain(lapsed) {
    ending.spawn(async move { session.kept.close().await });
  }

  for link in host.placed.externals.every() {
    ending.spawn(async move { link.end().await });
  }

  ending.join_all().await;
}

// How often the host sweeps its records and versions, reads its schedules, how long changes under the tools must
// have settled before it reads them again, and how an owed wake is tried.
#[derive(Clone, Copy)]
struct Beats {
  sweep: Duration,
  schedule: Duration,
  settle: Duration,
  wake: WakeTries,
}

// The host served: the tools face on the loopback port, the control socket at its path behind the issuer token and
// the tool sessions' tokens, and the console face on its own loopback port where it is opened.
async fn serve(
  (port, console): (u16, Option<Opened>),
  data_root: &Path,
  (control, issuer): (&Path, String),
  beats: Beats,
  host: Arc<Host>,
) -> ExitCode {
  let home = data_root.join("tool-host");
  let records = match Records::load(&home.join("sessions")) {
    Ok(records) => Arc::new(records),
    Err(why) => {
      return refused(format!("the session records in {} could not be read: {why}", home.display()));
    }
  };
  // The placed tools are read before the first request and watched for as long as the host serves.
  if let Err(why) = host.placed.watch(beats.settle) {
    return refused(format!("the placed tools could not be watched: {why}"));
  }

  let tcp = match TcpListener::bind(("127.0.0.1", port)).await {
    Ok(listener) => listener,
    Err(why) => return refused(format!("127.0.0.1:{port} could not be bound: {why}")),
  };
  let unix = match bind_control(control) {
    Ok(listener) => listener,
    Err(why) => {
      return refused(format!("the control socket {} could not be bound: {why}", control.display()));
    }
  };
  let console = match console {
    Some(opened) => match TcpListener::bind(("127.0.0.1", opened.port)).await {
      Ok(listener) => Some((listener, opened.screen)),
      Err(why) => return refused(format!("127.0.0.1:{} could not be bound: {why}", opened.port)),
    },
    None => None,
  };
  let asked_to_stop = stop_asked();
  let bound = tcp.local_addr().map_or(port, |address| address.port());
  let screen_at = console
    .as_ref()
    .and_then(|(listener, _)| listener.local_addr().ok())
    .map_or_else(String::new, |address| format!(", console 127.0.0.1:{}", address.port()));

  println!("mcp-host listening on 127.0.0.1:{bound}, control {}{screen_at}", control.display());
  let _ = io::stdout().flush();

  let pruning = records.clone();
  let sweeping = host.clone();

  // Lapsed records dropped on every beat, and what each kept ended, so a session whose issuer never took its
  // record back holds what its tools keep no longer than one beat past its `expiresAt`; and every retired
  // version nothing holds any longer taken away.
  tokio::spawn(async move {
    loop {
      for (_, gone) in pruning.prune() {
        gone.kept.close().await;
      }
      sweeping.placed.sweep();
      tokio::time::sleep(beats.sweep).await;
    }
  });

  // The asynchronous calls kept settled before the first request: a call the previous host lost ended so, and
  // every wake still owed asked again.
  let calls = Arc::new(AsyncCalls::new(data_root.to_path_buf(), beats.wake));

  calls.settle(&host);

  // Every schedule of the catalogue kept, those due called by the host itself.
  let schedules = Arc::new(Schedules::default());

  tokio::spawn(schedule::keep(host.clone(), schedules.clone(), beats.schedule));

  // The stop told everywhere it is waited for, and every event stream ended, so the control socket's serving lets
  // go of it.
  let stopping = host.clone();

  tokio::spawn(async move {
    asked_to_stop.await;
    stopping.drain.stop();
    stopping.placed.events.stop();
  });

  // Each face stops listening at the stop and ends once it has answered the requests it holds.
  let stop = || {
    let host = host.clone();

    async move { host.drain.stopping().await }
  };
  let sessions =
    axum::serve(tcp, server::tools(records.clone(), host.clone(), calls)).with_graceful_shutdown(stop());
  let screens = async {
    match console {
      Some((listener, screen)) => {
        let face = server::console(records.clone(), host.clone(), schedules.clone(), screen);

        axum::serve(listener, face).with_graceful_shutdown(stop()).await
      }
      None => Ok(()),
    }
  };
  let issuers =
    axum::serve(unix, server::control(records.clone(), host.clone(), schedules.clone(), issuer))
      .with_graceful_shutdown(stop());
  let (served, controlled, shown) = tokio::join!(sessions, issuers, screens);

  // What the faces do not hold is waited on too, however long it takes: an asynchronous call, a scheduled one.
  host.drain.emptied().await;
  end_what_is_kept(&records, &host).await;

  let _ = fs::remove_file(control);

  match served.and(controlled).and(shown) {
    Ok(()) => ExitCode::SUCCESS,
    Err(why) => refused(why),
  }
}

fn call(namespace: &str, tool: &str, host: Arc<Host>) -> ExitCode {
  let mut said = String::new();

  if let Err(why) = io::stdin().read_to_string(&mut said) {
    return refused(why);
  }

  let asked: Asked = match serde_json::from_str(&said) {
    Ok(asked) => asked,
    Err(why) => return refused(format!("what was handed is not a session and its arguments: {why}")),
  };
  let session = asked.session.issued_at(now_ms());
  let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
    Ok(runtime) => runtime,
    Err(why) => return refused(why),
  };

  // The external servers' tools are listed first, as the running host lists them.
  runtime.block_on(host.placed.refresh());

  // The record's namespaces, capabilities and grants hold here as they hold for a session's call.
  let Some(entry) = host.find(&session, namespace, tool) else {
    return refused(format!("unknown tool `{tool}` in `{namespace}` for this session"));
  };
  let answered = runtime.block_on(async {
    let answer = host.answer(&entry, &session, asked.arguments).await;

    session.kept.close().await;
    answer
  });

  match answered {
    Ok(answer) => {
      println!("{answer}");
      ExitCode::SUCCESS
    }
    Err(why) => refused(why.message),
  }
}
