//! `bmo-pocket`, which `bmo pocket <verb>` hands its words to (docs/api.md, 「The CLI」; docs/registry.md): a tool
//! put into the registry, taken out of it, or the registry listed. What it did goes to standard output and why it
//! refused to standard error; it ends 0 where it did what was asked, 1 where it refused, and 2 where its words do
//! not read.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use mcp_host::install::{self, Put};
use mcp_host::registry::Registry;

#[derive(Parser)]
#[command(
  name = "bmo-pocket",
  about = "The tool host's registry: put a tool, take one, list them."
)]
struct Cli {
  /// The data root whose `tool-host/` holds the registry.
  #[arg(long, env = "BMO_DATA_DIR", global = true)]
  data_root: Option<PathBuf>,
  #[command(subcommand)]
  verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
  /// Places a tool and registers it, or upgrades the tool registered under its name.
  Put {
    /// A local folder, a git address, or a bare tool name put from the source template.
    tool: String,
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
  },
  /// Takes the tool registered under the name out of the registry.
  Take {
    name: String,
    /// The repository whose own tool it is, which alone takes a service's tool.
    #[arg(long)]
    service: Option<String>,
  },
  /// Every registered tool.
  List {
    /// The pointers as they stand, as JSON.
    #[arg(long)]
    json: bool,
  },
}

fn main() -> ExitCode {
  let cli = Cli::parse();
  let data_root = cli.data_root.unwrap_or_else(|| {
    std::env::var_os("HOME")
      .map_or_else(|| PathBuf::from("/"), PathBuf::from)
      .join(".local/share/bmo/data")
  });
  let registry = Registry::of(&data_root);
  let mut said = Vec::new();
  let done = match cli.verb {
    Verb::Put {
      tool,
      allow,
      service,
      placed,
      source,
    } => {
      let asked = Put {
        tool,
        allow,
        service,
        placed,
        source,
      };

      install::put(&registry, &asked, &mut said)
    }
    Verb::Take { name, service } => install::take(&registry, &name, service.as_deref(), &mut said),
    Verb::List { json } => install::list(&registry, json, &mut said),
  };

  for line in &said {
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
