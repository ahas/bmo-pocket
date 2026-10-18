//! The tool host's binary (docs/mcp-host.md). Its verbs:
//!
//! - `serve`: the host itself — every namespace on the loopback port, behind each session's bearer
//!   token, and the control socket the launchers issue and revoke tokens on.
//! - `call <namespace> <tool>`: one call of one placed tool as a session would make it, the
//!   session record and the arguments read from standard input as `{"session": …, "arguments": …}`
//!   and the answer written to standard output as the protocol writes it. It is how a person or a
//!   test asks the tools without a session, and it reaches exactly what the host serves.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use bmo_tool::Session;
use clap::{Args, Parser, Subcommand};
use mcp_host::handler::Host;
use mcp_host::lock::{Accounts, Lock};
use mcp_host::placed::Placed;
use mcp_host::records::Records;
use mcp_host::running::sockets_beside;
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

/// What a call reaches beyond the tool: the grant lock's account service and the variables passed on.
#[derive(Args)]
struct Reach {
  /// The account service's base address, which the grant lock asks; with none, nothing is checked.
  #[arg(long, env = "BMO_ACCOUNTS_URL")]
  accounts: Option<String>,
  /// The file holding the bearer token the account service is asked with.
  #[arg(long, env = "BMO_ACCOUNTS_TOKEN_FILE")]
  accounts_token_file: Option<PathBuf>,
  /// The grant lock's switch: `off` turns it off.
  #[arg(long, env = "BMO_GRANT_LOCK", default_value = "on")]
  grant_lock: String,
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
    /// The placed tools' folder; `tools/` beside the `bin/` this binary stands in by default.
    #[arg(long)]
    tools: Option<PathBuf>,
  },
}

#[derive(Deserialize)]
struct Asked {
  session: Session,
  #[serde(default)]
  arguments: Value,
}

fn refused(why: impl std::fmt::Display) -> ExitCode {
  eprintln!("mcp-host: {why}");
  ExitCode::FAILURE
}

impl Reach {
  // The lock and the variables passed on, the account service's token read from its file.
  fn host(self, placed: Arc<Placed>) -> Result<Host, String> {
    let token = match &self.accounts_token_file {
      Some(file) => Some(
        fs::read_to_string(file)
          .map_err(|why| {
            format!("the account service's token {} could not be read: {why}", file.display())
          })?
          .trim()
          .to_owned(),
      ),
      None => None,
    };
    let lock = Lock {
      accounts: self.accounts.map(|base| Accounts { base, token }),
      off: self.grant_lock.trim() == "off",
      client: reqwest::Client::new(),
    };

    Ok(Host {
      lock,
      placed,
      pass_env: self.pass_env,
    })
  }
}

fn main() -> ExitCode {
  match Cli::parse().verb {
    Verb::Serve {
      port,
      data_root,
      control,
      sweep_every_ms,
      reach,
      tools,
      settle_ms,
    } => {
      let data_root = data_root.unwrap_or_else(|| home().join(".local/share/bmo/data"));
      let control = control.unwrap_or_else(|| runtime_dir().join("bmo/mcp-host.sock"));
      let placed = Arc::new(placed(tools, &control));
      let host = match reach.host(placed) {
        Ok(host) => Arc::new(host),
        Err(why) => return refused(why),
      };

      match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(serve(
          port,
          &data_root,
          &control,
          Duration::from_millis(sweep_every_ms),
          host,
          Duration::from_millis(settle_ms),
        )),
        Err(why) => refused(why),
      }
    }
    Verb::Call {
      namespace,
      tool,
      reach,
      tools,
    } => {
      let placed = placed(tools, &runtime_dir().join("bmo/mcp-host.sock"));

      placed.reread();

      match reach.host(Arc::new(placed)) {
        Ok(host) => call(&namespace, &tool, Arc::new(host)),
        Err(why) => refused(why),
      }
    }
  }
}

fn home() -> PathBuf {
  std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

// The user's runtime directory, where the sockets of this machine's services stand.
fn runtime_dir() -> PathBuf {
  std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

// The placed tools' catalogue: the folder named, or `tools/` beside the `bin/` this binary stands in, its
// sockets beside the control socket.
fn placed(tools: Option<PathBuf>, control: &Path) -> Placed {
  let tools = tools
    .or_else(|| std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.parent()?.join("tools"))));

  Placed::new(tools, sockets_beside(control))
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

async fn serve(
  port: u16,
  data_root: &Path,
  control: &Path,
  sweep_every: Duration,
  host: Arc<Host>,
  settle: Duration,
) -> ExitCode {
  let home = data_root.join("tool-host");
  let records = match Records::load(&home.join("sessions")) {
    Ok(records) => Arc::new(records),
    Err(why) => {
      return refused(format!("the session records in {} could not be read: {why}", home.display()));
    }
  };
  // The placed tools are read before the first request and watched for as long as the host serves.
  if let Err(why) = host.placed.watch(settle) {
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

  // Lapsed records dropped on every beat, and what each kept ended, so a run that ended without its
  // record being taken back holds its browser no longer than one beat past its deadline.
  tokio::spawn(async move {
    loop {
      for (_, gone) in pruning.prune() {
        gone.kept.close().await;
      }
      tokio::time::sleep(sweep_every).await;
    }
  });

  let sessions =
    axum::serve(tcp, server::tools(records.clone(), host.clone())).with_graceful_shutdown(stopped());
  let launchers = axum::serve(unix, server::control(records, host)).with_graceful_shutdown(stopped());
  let (served, controlled) = tokio::join!(sessions, launchers);
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
  // The grant lock holds here as it holds for a session's call.
  let Some(entry) = host.find(&asked.session, namespace, tool) else {
    return refused(format!("unknown tool `{tool}` in `{namespace}` for this session"));
  };
  let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
    Ok(runtime) => runtime,
    Err(why) => return refused(why),
  };
  let answered = runtime.block_on(async {
    let answer = host.answer(&entry, &asked.session, asked.arguments).await;

    asked.session.kept.close().await;
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
