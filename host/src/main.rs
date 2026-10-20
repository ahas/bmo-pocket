//! The tool host's binary (docs/mcp-host.md). Its verbs:
//!
//! - `serve`: the host itself — every namespace on the loopback port, behind each session's bearer
//!   token, and the control socket tool sessions are issued and revoked on.
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
use mcp_host::handler::Host;
use mcp_host::placed::Placed;
use mcp_host::records::{Records, now_ms};
use mcp_host::registry::Registry;
use mcp_host::running::sockets_beside;
use mcp_host::schedule::{self, Schedules};
use mcp_host::server;
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
    }
  }
}

fn main() -> ExitCode {
  match Cli::parse().verb {
    Verb::Serve {
      port,
      data_root,
      control,
      sweep_every_ms,
      schedule_every_ms,
      reach,
      tools,
      settle_ms,
    } => {
      let data_root = data_root.unwrap_or_else(default_data_root);
      let control = control.unwrap_or_else(|| runtime_dir().join("bmo/mcp-host.sock"));
      let placed = Arc::new(placed(tools, &data_root, &control));
      let host = Arc::new(reach.host(placed));

      match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(serve(
          port,
          &data_root,
          &control,
          Beats {
            sweep: Duration::from_millis(sweep_every_ms),
            schedule: Duration::from_millis(schedule_every_ms),
            settle: Duration::from_millis(settle_ms),
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

async fn stopped() {
  let (Ok(mut term), Ok(mut interrupt)) =
    (signal(SignalKind::terminate()), signal(SignalKind::interrupt()))
  else {
    return std::future::pending().await;
  };

  tokio::select! {
    _ = term.recv() => {}
    _ = interrupt.recv() => {}
  }
}

// How often the host sweeps its records and versions, reads its schedules, and how long changes under the tools
// must have settled before it reads them again.
#[derive(Clone, Copy)]
struct Beats {
  sweep: Duration,
  schedule: Duration,
  settle: Duration,
}

async fn serve(port: u16, data_root: &Path, control: &Path, beats: Beats, host: Arc<Host>) -> ExitCode {
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
  let bound = tcp.local_addr().map_or(port, |address| address.port());

  println!("mcp-host listening on 127.0.0.1:{bound}, control {}", control.display());
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

  // Every schedule of the catalogue kept, those due called by the host itself.
  let schedules = Arc::new(Schedules::default());

  tokio::spawn(schedule::keep(host.clone(), schedules.clone(), beats.schedule));

  let sessions =
    axum::serve(tcp, server::tools(records.clone(), host.clone())).with_graceful_shutdown(stopped());
  let issuers =
    axum::serve(unix, server::control(records, host, schedules)).with_graceful_shutdown(stopped());
  let (served, controlled) = tokio::join!(sessions, issuers);
  let _ = fs::remove_file(control);

  match served.and(controlled) {
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
  // The record's namespaces, capabilities and grants hold here as they hold for a session's call.
  let Some(entry) = host.find(&session, namespace, tool) else {
    return refused(format!("unknown tool `{tool}` in `{namespace}` for this session"));
  };
  let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
    Ok(runtime) => runtime,
    Err(why) => return refused(why),
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
