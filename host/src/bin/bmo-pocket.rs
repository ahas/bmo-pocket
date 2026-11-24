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
    /// The key of the shared app a received personal tool belongs to.
    #[arg(long, requires = "owner")]
    app: Option<String>,
  },
  /// A company's catalogue of the personal apps its people share (docs/catalogue.md).
  #[command(arg_required_else_help = true)]
  Catalogue {
    #[command(subcommand)]
    verb: CatalogueVerb,
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

#[derive(Subcommand)]
enum CatalogueVerb {
  /// Shares a version of a maker's item, its kind read off the bundle, declaring its two settings on the delegation
  /// chain.
  Share {
    #[arg(long)]
    company: String,
    #[arg(long)]
    maker: String,
    /// The maker's home unit as they share it.
    #[arg(long)]
    unit: String,
    /// The version the maker's PC placed the app's build under.
    #[arg(long)]
    version: String,
    /// Whom it is shared with, as a JSON array of targets.
    #[arg(long)]
    targets: String,
    /// The source bundle's file, `{"files": {<path>: <text>}}`.
    #[arg(long)]
    bundle: PathBuf,
    /// Whether a head must approve the version before anybody installs it.
    #[arg(long)]
    review_required: bool,
    /// The maker's language, which a plain text the item names no language for is read in.
    #[arg(long)]
    language: Option<String>,
  },
  /// One locale's text of an item set, or cleared.
  Text {
    #[arg(long)]
    company: String,
    id: String,
    #[arg(long)]
    locale: String,
    #[arg(long, required_unless_present = "clear", requires = "description")]
    title: Option<String>,
    #[arg(long, required_unless_present = "clear", requires = "title")]
    description: Option<String>,
    /// Whether a translation wrote it.
    #[arg(long, conflicts_with = "clear")]
    machine: bool,
    #[arg(long, conflicts_with_all = ["title", "description"])]
    clear: bool,
  },
  /// Every app's record, and with `--reader` the moment that reader last looked.
  List {
    #[arg(long)]
    company: String,
    #[arg(long)]
    reader: Option<String>,
  },
  /// One app's record.
  Show {
    #[arg(long)]
    company: String,
    id: String,
  },
  /// A version's source bundle, checked against the hash recorded as it was shared; the newest by default.
  Source {
    #[arg(long)]
    company: String,
    id: String,
    #[arg(long)]
    version: Option<String>,
  },
  /// The app withdrawn: no new install from now.
  Withdraw {
    #[arg(long)]
    company: String,
    id: String,
  },
  /// A version's review written.
  Review {
    #[arg(long)]
    company: String,
    id: String,
    #[arg(long)]
    version: String,
    /// `written` or `failed`.
    #[arg(long)]
    state: String,
    /// The file holding the report, or why none was written.
    #[arg(long)]
    report: PathBuf,
  },
  /// A version approved by a head.
  Approve {
    #[arg(long)]
    company: String,
    id: String,
    #[arg(long)]
    version: String,
    #[arg(long)]
    by: String,
  },
  /// A receiver's copy kept.
  Install {
    #[arg(long)]
    company: String,
    id: String,
    #[arg(long)]
    account: String,
    #[arg(long)]
    version: String,
    /// `self` or `unit`.
    #[arg(long)]
    how: String,
  },
  /// A receiver's copy let go.
  Uninstall {
    #[arg(long)]
    company: String,
    id: String,
    #[arg(long)]
    account: String,
  },
  /// Every app the leaver owns passed to another account.
  Pass {
    #[arg(long)]
    company: String,
    #[arg(long)]
    from: String,
    #[arg(long)]
    to: String,
  },
  /// The request to make an app a platform app.
  Promote {
    #[arg(long)]
    company: String,
    id: String,
    #[arg(long)]
    by: String,
    #[arg(long, default_value = "")]
    note: String,
  },
  /// Every company's requests to make an app a platform app.
  Promotions,
  /// The reader's looking at what was shared with them kept as now.
  Seen {
    #[arg(long)]
    company: String,
    #[arg(long)]
    account: String,
  },
}

// One catalogue verb: what it keeps printed as JSON, or why it refused.
fn catalogue_verb(data_root: &Path, verb: CatalogueVerb) -> ExitCode {
  use mcp_host::catalogue::{self as kept, Bundle, Catalogue, Share};

  let of = |company: &str| Catalogue::of(data_root, company);
  let record = |done: Result<kept::Record, String>| {
    done.and_then(|record| serde_json::to_value(record).map_err(|why| why.to_string()))
  };
  let mut said = Vec::new();
  let done: Result<serde_json::Value, String> = match verb {
    CatalogueVerb::Share {
      company,
      maker,
      unit,
      version,
      targets,
      bundle,
      review_required,
      language,
    } => (|| {
      let catalogue = of(&company)?;
      let targets: serde_json::Value =
        serde_json::from_str(&targets).map_err(|why| format!("the targets do not read: {why}"))?;
      let bundle: Bundle = std::fs::read_to_string(&bundle)
        .map_err(|why| why.to_string())
        .and_then(|text| serde_json::from_str(&text).map_err(|why| why.to_string()))
        .map_err(|why| format!("the bundle {} does not read: {why}", bundle.display()))?;
      let chain = mcp_host::chain::Chain::named(data_root);
      let asked = Share {
        maker: &maker,
        unit: &unit,
        version: &version,
        targets,
        bundle,
        review_required,
        language: language.as_deref(),
      };

      record(kept::share(&catalogue, chain.as_ref(), &asked, &mut said))
    })(),
    CatalogueVerb::Text {
      company,
      id,
      locale,
      title,
      description,
      machine,
      clear,
    } => of(&company).and_then(|catalogue| {
      let text = match (clear, title.as_deref(), description.as_deref()) {
        (false, Some(title), Some(description)) => Some((title, description)),
        _ => None,
      };

      record(kept::set_text(&catalogue, &id, &locale, text, machine))
    }),
    CatalogueVerb::List { company, reader } => of(&company).map(|catalogue| {
      let apps = catalogue.list();
      let seen = reader.as_deref().and_then(|reader| catalogue.seen_at(reader));

      serde_json::json!({ "apps": apps, "seenAt": seen })
    }),
    CatalogueVerb::Show { company, id } => of(&company).and_then(|catalogue| {
      record(
        catalogue
          .get(&id)
          .ok_or_else(|| format!("the catalogue of {company} holds no app {id}")),
      )
    }),
    CatalogueVerb::Source {
      company,
      id,
      version,
    } => of(&company).and_then(|catalogue| catalogue.source(&id, version.as_deref())),
    CatalogueVerb::Withdraw { company, id } => {
      of(&company).and_then(|catalogue| record(kept::withdraw(&catalogue, &id)))
    }
    CatalogueVerb::Review {
      company,
      id,
      version,
      state,
      report,
    } => of(&company).and_then(|catalogue| {
      let text = std::fs::read_to_string(&report).map_err(|why| format!("{}: {why}", report.display()))?;

      record(kept::review(&catalogue, &id, &version, &state, &text))
    }),
    CatalogueVerb::Approve {
      company,
      id,
      version,
      by,
    } => of(&company).and_then(|catalogue| record(kept::approve(&catalogue, &id, &version, &by))),
    CatalogueVerb::Install {
      company,
      id,
      account,
      version,
      how,
    } => {
      of(&company).and_then(|catalogue| record(kept::install(&catalogue, &id, &account, &version, &how)))
    }
    CatalogueVerb::Uninstall {
      company,
      id,
      account,
    } => of(&company).and_then(|catalogue| record(kept::uninstall(&catalogue, &id, &account))),
    CatalogueVerb::Pass { company, from, to } => of(&company)
      .and_then(|catalogue| kept::pass(&catalogue, &from, &to))
      .map(|passed| serde_json::json!(passed)),
    CatalogueVerb::Promote {
      company,
      id,
      by,
      note,
    } => of(&company).and_then(|catalogue| record(kept::promote(&catalogue, &id, &by, &note))),
    CatalogueVerb::Promotions => Ok(serde_json::Value::Array(kept::promotions(data_root))),
    CatalogueVerb::Seen { company, account } => of(&company)
      .and_then(|catalogue| catalogue.seen(&account))
      .map(|at| serde_json::json!({ "at": at })),
  };

  for line in &said {
    eprintln!("bmo-pocket: {line}");
  }

  match done {
    Ok(value) => {
      println!("{value}");
      ExitCode::SUCCESS
    }
    Err(why) => {
      eprintln!("bmo-pocket: {why}");
      ExitCode::FAILURE
    }
  }
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
    Verb::Catalogue { verb } => catalogue_verb(&data_root, verb),
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
      app,
    } => {
      if let Some(owner) = owner {
        let mut said = Vec::new();
        let folder = PathBuf::from(tool.unwrap_or_default());
        let done = install::put_owned(
          &Registry::of(&data_root),
          &owner,
          &folder,
          (version.as_deref(), app.as_deref()),
          &mut said,
        );

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
