//! `bmo-word-chain`, which `bmo word-chain <verb>` hands its words to (docs/word-chain.md): the word book
//! installed under the data root, and a word looked up in it. What it did goes to standard output and why it
//! refused to standard error; it ends 0 where it did what was asked, 1 where it refused, and 2 where its words do
//! not read.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use word_chain::install::{book_path, install, look_up};

#[derive(Parser)]
#[command(
  name = "bmo-word-chain",
  about = "The word chain: its word book installed and a word looked up."
)]
struct Cli {
  /// The data root whose `word-chain/ko.sqlite` is the word book.
  #[arg(long, env = "BMO_DATA_DIR", global = true)]
  data_root: Option<PathBuf>,
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
}

fn main() -> ExitCode {
  let cli = Cli::parse();
  let data_root = cli.data_root.unwrap_or_else(|| {
    std::env::var_os("HOME")
      .map_or_else(|| PathBuf::from("/"), PathBuf::from)
      .join(".local/share/bmo/data")
  });
  let book = book_path(&data_root);
  let done = match cli.verb {
    Verb::Install { from, nikl_from } => match tokio::runtime::Runtime::new() {
      Ok(runtime) => runtime.block_on(install(&book, from.as_deref(), nikl_from.as_deref())),
      Err(why) => Err(why.to_string()),
    },
    Verb::Word { word } => look_up(&book, &word),
  };

  match done {
    Ok(lines) => {
      for line in lines {
        println!("{line}");
      }

      ExitCode::SUCCESS
    }
    Err(why) => {
      eprintln!("bmo-word-chain: {why}");
      ExitCode::FAILURE
    }
  }
}
