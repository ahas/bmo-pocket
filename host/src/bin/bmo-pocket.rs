//! `bmo-pocket`, which `bmo pocket <verb>` hands its registry words to (docs/api.md, 「The CLI」; docs/registry.md)
//! and which declares the `tool` nouns `bmo tool <verb>` hands its own: the host's catalogue listed, described and
//! called, and its site tools read. It also serves the harness plugin's MCP server on standard input and output
//! (docs/api.md, 「The harness plugin」). A verb writes what it did on standard output and why it refused on standard
//! error; it ends 0 where it did what was asked, 1 where it refused, and 2 where its words do not read.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bmo_pocket_client::{Pocket, default_socket};
use clap::{CommandFactory as _, Parser, Subcommand};
use mcp_host::command;
use mcp_host::install::{self, Put};
use mcp_host::registry::Registry;

#[derive(Parser)]
#[command(
  name = "bmo-pocket",
  about = "The tool host's registry and catalogue: put a tool, take one, list the registry, declare a screen-only app, list, describe and call a catalogue entry, read its site tools; and its harness plugin's MCP server."
)]
struct Cli {
  /// The data root whose `tool-host/` holds the registry and the issuer token.
  #[arg(long, env = "BMO_DATA_DIR", global = true)]
  data_root: Option<PathBuf>,
  /// The file the issuer token is kept in; `tool-host/issuer-token` under the data root by default.
  #[arg(long, global = true)]
  issuer_token_file: Option<PathBuf>,
  #[command(subcommand)]
  verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
  /// Every noun this command declares, one a line.
  Nouns,
  /// The host's catalogue: the entries listed, one described, one called, and the site tools read.
  #[command(arg_required_else_help = true)]
  Tool {
    #[command(subcommand)]
    noun: command::Noun,
  },
  /// Places a tool and registers it, or upgrades the tool registered under its name; with `--mcp`, registers an
  /// external MCP server by its registration file.
  Put {
    /// A local folder, a git address, or a bare tool name put from the source template.
    #[arg(required_unless_present = "mcp", conflicts_with = "mcp")]
    tool: Option<String>,
    /// An external MCP server's registration file, registered in place of a tool.
    #[arg(long, conflicts_with = "placed")]
    mcp: Option<PathBuf>,
    /// A capability or grant the tool may ask for; given again for each.
    #[arg(long)]
    allow: Vec<String>,
    /// The repository whose own tool this is.
    #[arg(long)]
    service: Option<String>,
    /// The folder is one a deployment already built: it is placed whole, its build not run.
    #[arg(long)]
    placed: bool,
    /// The source template a bare name is put from, `{repo}` in it standing for `bmo-<name>`.
    #[arg(long, env = "BMO_POCKET_SOURCE")]
    source: Option<String>,
    /// The account whose personal tool's description copy the folder holds, put in that owner's registry.
    #[arg(long, conflicts_with_all = ["mcp", "service", "placed"])]
    owner: Option<String>,
    /// The version an owner's personal tool is placed as, the name its owner's machine placed it under.
    #[arg(long, requires = "owner")]
    version: Option<String>,
  },
  /// Takes the tool registered under the name out of the registry, or the owner's personal tool out of theirs.
  Take {
    name: String,
    /// The repository whose own tool it is, which alone takes a service's tool.
    #[arg(long)]
    service: Option<String>,
    /// The account whose personal tool it is, taken from that owner's registry.
    #[arg(long, conflicts_with = "service")]
    owner: Option<String>,
  },
  /// Declares a screen-only app's two settings on the delegation chain from a file holding its `[app]` table alone;
  /// places nothing in the registry.
  App {
    /// The declaration file: a TOML file holding the `[app]` table alone.
    file: PathBuf,
  },
  /// Every registered tool.
  List {
    /// The pointers as they stand, as JSON.
    #[arg(long)]
    json: bool,
    /// The account whose personal tools are listed, in place of the platform's.
    #[arg(long)]
    owner: Option<String>,
  },
  /// Says whether a stop of the running host would wait on any call: ends 0 where it would not, and 1 naming how many.
  Quiet,
  /// Serves the harness plugin's MCP server on standard input and output, under one tool session.
  Mcp {
    /// The control socket; `BMO_MCP_HOST_SOCKET`, or `bmo/mcp-host.sock` under the user's runtime directory.
    #[arg(long, env = "BMO_MCP_HOST_SOCKET")]
    socket: Option<PathBuf>,
    /// The tool session's token every call is made with.
    #[arg(long, env = "BMO_POCKET_TOKEN", hide_env_values = true)]
    token: String,
  },
  /// The relay the host hands a personal tool's call to: the call on standard input, relayed to the owner's runner,
  /// its answer on standard output, its errors and its exit code (docs/api.md, 「The runner」).
  #[command(hide = true)]
  Relay {
    /// What the host tells the relay of the call, as JSON: the owner, the tool, its version, its run words and the
    /// runner.
    request: String,
  },
}

// One personal tool's call relayed to its owner's runner, ending as the relay answers.
fn relay(request: &str) -> ExitCode {
  let request: mcp_host::relay::Request = match serde_json::from_str(request) {
    Ok(request) => request,
    Err(why) => {
      eprintln!("bmo-pocket relay: the request does not read: {why}");
      return ExitCode::from(2);
    }
  };
  let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
    Ok(runtime) => runtime,
    Err(why) => {
      eprintln!("bmo-pocket relay: {why}");
      return ExitCode::FAILURE;
    }
  };

  ExitCode::from(runtime.block_on(mcp_host::relay::relay(
    request,
    tokio::io::stdin(),
    tokio::io::stdout(),
    tokio::io::stderr(),
  )))
}

// The harness plugin's MCP server, served until the harness closes it.
fn mcp(socket: Option<PathBuf>, token: String) -> ExitCode {
  let client = Pocket::at(socket.unwrap_or_else(default_socket), token);
  let served = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .map_err(|why| why.to_string())
    .and_then(|runtime| runtime.block_on(bmo_pocket_plugin::serve_stdio(client)));

  match served {
    Ok(()) => ExitCode::SUCCESS,
    Err(why) => {
      eprintln!("bmo-pocket: {why}");
      ExitCode::FAILURE
    }
  }
}

// One `tool` noun, run to its answer against the host the issuer token opens.
fn tool(noun: command::Noun, data_root: &Path, issuer_token_file: Option<&Path>) -> ExitCode {
  let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
    Ok(runtime) => runtime,
    Err(why) => {
      eprintln!("bmo-pocket: {why}");
      return ExitCode::FAILURE;
    }
  };

  match runtime.block_on(command::run(noun, data_root, issuer_token_file)) {
    Ok(answered) => {
      for line in answered.lines {
        println!("{line}");
      }

      for note in answered.notes {
        eprintln!("bmo-pocket: {note}");
      }

      if answered.failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
    }
    Err(why) => {
      eprintln!("bmo-pocket: {why}");
      ExitCode::FAILURE
    }
  }
}

// Whether a stop of the running host would wait on any call, said on standard output and by the exit.
fn quiet(data_root: &Path, issuer_token_file: Option<&Path>) -> ExitCode {
  let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
    Ok(runtime) => runtime,
    Err(why) => {
      eprintln!("bmo-pocket: {why}");
      return ExitCode::FAILURE;
    }
  };
  let (quiet, said) = runtime.block_on(command::quiet(data_root, issuer_token_file));

  println!("{said}");

  if quiet { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn default_data_root() -> PathBuf {
  std::env::var_os("HOME")
    .map_or_else(|| PathBuf::from("/"), PathBuf::from)
    .join(".local/share/bmo/data")
}

fn finish(done: Result<(), String>, said: &[String]) -> ExitCode {
  for line in said {
    println!("{line}");
  }

  match done {
    Ok(()) => ExitCode::SUCCESS,
    Err(why) => {
      eprintln!("bmo-pocket: {why}");
      ExitCode::FAILURE
    }
  }
}

fn main() -> ExitCode {
  // The `tool` noun alone writes its usage, which names the four, on standard error and ends 2, as `tool --help`
  // writes it on standard output. Without this, clap answers a required subcommand's absence in its own words.
  let words: Vec<std::ffi::OsString> = std::env::args_os().collect();

  if words.len() == 2 && words[1] == "tool" {
    if let Err(why) = Cli::command().try_get_matches_from(["bmo-pocket", "tool", "--help"]) {
      eprint!("{why}");
    }

    return ExitCode::from(2);
  }

  let cli = Cli::parse();
  let data_root = cli.data_root.clone().unwrap_or_else(default_data_root);

  match cli.verb {
    Verb::Nouns => {
      for noun in command::NOUNS {
        println!("{noun}");
      }

      ExitCode::SUCCESS
    }
    Verb::Tool { noun } => tool(noun, &data_root, cli.issuer_token_file.as_deref()),
    Verb::Quiet => quiet(&data_root, cli.issuer_token_file.as_deref()),
    Verb::Mcp { socket, token } => mcp(socket, token),
    Verb::Relay { request } => relay(&request),
    Verb::Put {
      tool,
      mcp,
      allow,
      service,
      placed,
      source,
      owner,
      version,
    } => {
      if let Some(owner) = owner {
        let mut said = Vec::new();
        let folder = PathBuf::from(tool.unwrap_or_default());
        let done =
          install::put_owned(&Registry::of(&data_root), &owner, &folder, version.as_deref(), &mut said);

        return finish(done, &said);
      }

      let asked = Put {
        tool: mcp.as_ref().map_or_else(|| tool.unwrap_or_default(), |file| file.display().to_string()),
        allow,
        service,
        placed,
        source,
        chain: mcp_host::chain::Chain::named(&data_root),
      };
      let mut said = Vec::new();
      let registry = Registry::of(&data_root);
      let done = match mcp {
        Some(_) => install::register_server(&registry, &asked, &mut said),
        None => install::put(&registry, &asked, &mut said),
      };

      finish(done, &said)
    }
    Verb::Take {
      name,
      service,
      owner,
    } => {
      let mut said = Vec::new();
      let registry = Registry::of(&data_root);
      let done = match owner {
        Some(owner) => install::take_owned(&registry, &owner, &name, &mut said),
        None => install::take(&registry, &name, service.as_deref(), &mut said),
      };

      finish(done, &said)
    }
    Verb::App { file } => {
      let mut said = Vec::new();
      let chain = mcp_host::chain::Chain::named(&data_root);
      let done = install::app(&file, chain.as_ref(), &mut said);

      finish(done, &said)
    }
    Verb::List { json, owner } => {
      let mut said = Vec::new();
      let registry = Registry::of(&data_root);
      let done = match owner {
        Some(owner) => install::list_owned(&registry, &owner, json, &mut said),
        None => install::list(&registry, json, &mut said),
      };

      finish(done, &said)
    }
  }
}
