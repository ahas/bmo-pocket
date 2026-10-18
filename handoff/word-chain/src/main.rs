//! `bmo-word-chain`, which `bmo word-chain <verb>` hands its words to (docs/word-chain.md): the word book
//! installed and a word looked up, the service run, the tool's one call answered, and a game opened or stopped
//! through the service's control routes. What it did goes to standard output and why it refused to standard
//! error; it ends 0 where it did what was asked, 1 where it refused, and 2 where its words do not read.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::Value;
use word_chain::config::Config;
use word_chain::install::{book_path, install, look_up};
use word_chain::tool;

#[derive(Parser)]
#[command(
  name = "bmo-word-chain",
  about = "The word chain: its word book, its service and its tool."
)]
struct Cli {
  /// The data root whose `word-chain/ko.sqlite` is the word book and whose `env/word-chain.toml` the
  /// configuration.
  #[arg(long, env = "BMO_DATA_DIR", global = true)]
  data_root: Option<PathBuf>,
  /// The configuration, where it is not the data root's.
  #[arg(long, global = true)]
  config: Option<PathBuf>,
  #[command(subcommand)]
  verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
  /// The word book installed out of KKuTu's word list and the NIKL dictionaries.
  Install {
    /// KKuTu's dump read from this file rather than fetched.
    #[arg(long)]
    from: Option<PathBuf>,
    /// The NIKL dictionaries read from a mirror of their downloads rather than fetched.
    #[arg(long)]
    nikl_from: Option<PathBuf>,
  },
  /// A word looked up in the book: whether it stands, what follows it and what it means.
  Word { word: String },
  /// The service: the games, the receiver's events read, and the control routes answered.
  Serve,
  /// Ends 0 once the service the configuration names answers on its control routes.
  Health,
  /// One call of the tool host's protocol, read on standard input and answered on standard output.
  Tool,
  /// A game opened in a room through the service.
  #[command(allow_negative_numbers = true)]
  Start {
    #[arg(long)]
    chat: i64,
    #[arg(long)]
    starter: Option<i64>,
    #[arg(long)]
    starter_name: Option<String>,
    /// Whether the robot plays, and how hard: off, easy, normal or hard.
    #[arg(long)]
    robot: Option<String>,
    #[arg(long)]
    seconds: Option<i64>,
    /// The word the asker already gave to open with.
    #[arg(long)]
    word: Option<String>,
    #[arg(long)]
    no_loanwords: bool,
    #[arg(long)]
    manner: bool,
  },
  /// The game of the person who asked stopped, with its scores so far; with nobody named, the room's only game.
  #[command(allow_negative_numbers = true)]
  Stop {
    #[arg(long)]
    chat: i64,
    #[arg(long)]
    player: Option<i64>,
  },
}

fn refused(why: impl std::fmt::Display) -> ExitCode {
  eprintln!("bmo-word-chain: {why}");
  ExitCode::FAILURE
}

fn said(lines: Result<Vec<String>, String>) -> ExitCode {
  match lines {
    Ok(lines) => {
      for line in lines {
        println!("{line}");
      }

      ExitCode::SUCCESS
    }
    Err(why) => refused(why),
  }
}

fn main() -> ExitCode {
  let cli = Cli::parse();
  let data_root = cli.data_root.unwrap_or_else(|| {
    std::env::var_os("HOME")
      .map_or_else(|| PathBuf::from("/"), PathBuf::from)
      .join(".local/share/bmo/data")
  });
  let config = || Config::read(&cli.config.clone().unwrap_or_else(|| Config::deployed(&data_root)));
  let runtime = match tokio::runtime::Runtime::new() {
    Ok(runtime) => runtime,
    Err(why) => return refused(why),
  };

  match cli.verb {
    Verb::Install { from, nikl_from } => {
      said(runtime.block_on(install(&book_path(&data_root), from.as_deref(), nikl_from.as_deref())))
    }
    Verb::Word { word } => said(look_up(&book_path(&data_root), &word)),
    Verb::Serve => match config() {
      Ok(config) => match runtime.block_on(word_chain::serve::serve(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => refused(why),
      },
      Err(why) => refused(why),
    },
    Verb::Health => {
      let answered = config().and_then(|config| {
        runtime.block_on(async {
          let reply = reqwest::Client::new()
            .get(config.control().url("/health"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|why| format!("the service does not answer at {}: {why}", config.listen))?;

          if reply.status().is_success() {
            Ok(())
          } else {
            Err(format!("the service answered {}", reply.status()))
          }
        })
      });

      match answered {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => refused(why),
      }
    }
    Verb::Tool => {
      let mut input = String::new();

      if let Err(why) = std::io::stdin().read_to_string(&mut input) {
        return refused(why);
      }

      let answered = serde_json::from_str::<Value>(&input)
        .map_err(|why| format!("the call does not read: {why}"))
        .and_then(|handed| runtime.block_on(tool::answer(&handed)));

      match answered {
        Ok(answer) => {
          println!("{answer}");
          ExitCode::SUCCESS
        }
        Err(why) => refused(why),
      }
    }
    Verb::Start {
      chat,
      starter,
      starter_name,
      robot,
      seconds,
      word,
      no_loanwords,
      manner,
    } => {
      let arguments = serde_json::json!({
        "bmo": robot, "starter": starter, "starter_name": starter_name, "seconds": seconds, "word": word,
        "no_loanwords": no_loanwords, "manner": manner,
      });
      let asked = config().and_then(|config| {
        let body = tool::start_body(&arguments, chat)?;

        runtime.block_on(tool::ask(&config.control(), "/word-chain/start", &body))
      });

      said(asked.map(|line| vec![line]))
    }
    Verb::Stop { chat, player } => {
      let arguments = serde_json::json!({ "player": player });
      let asked = config().and_then(|config| {
        runtime.block_on(tool::ask(
          &config.control(),
          "/word-chain/stop",
          &tool::stop_body(&arguments, chat),
        ))
      });

      said(asked.map(|line| vec![line]))
    }
  }
}
