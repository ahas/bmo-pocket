//! What a game of the word chain is played by (docs/word-chain.md): `policy/word-chain.toml`, compiled into the
//! binary and read on first use. The Korean words a room says to a game and the language's own rules stand there
//! as data; the code reads them by key.

use std::sync::LazyLock;

use serde::Deserialize;

/// The policy, read once.
pub static WORD_CHAIN: LazyLock<WordChainPlay> = LazyLock::new(|| {
  toml::from_str(include_str!("../policy/word-chain.toml"))
    .unwrap_or_else(|why| panic!("policy/word-chain.toml does not read: {why}"))
});

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WordChainPlay {
  pub words: RoomWords,
  pub times: Times,
  pub rounds: Rounds,
  pub points: Points,
  pub names: Names,
  pub initial_sound: Vec<InitialSound>,
  pub install: Install,
}

/// One turn of the initial-sound rule: an initial consonant read as another before the vowels named, each by its
/// index in the Unicode Hangul syllable block.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialSound {
  pub from: u32,
  pub to: u32,
  pub vowels: Vec<u32>,
}

/// What the word book is installed from.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Install {
  pub kkutu: Source,
  pub nikl: Dictionaries,
}

/// One source the book is read out of: its repository, the commit it is read at and its licence.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
  pub repository: String,
  pub commit: String,
  pub licence: String,
}

/// The dictionaries a word's meaning is read out of, and how they are read.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dictionaries {
  pub repository: String,
  pub commit: String,
  pub licence: String,
  pub stdict_parts: usize,
  pub stdict_last: String,
  pub krdict_parts: usize,
  pub noun: String,
  pub off_standard: Vec<String>,
  pub proper_fields: Vec<String>,
  pub no_field: String,
  pub meaning_chars: usize,
}

/// The words a room says to a standing game.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomWords {
  pub join: Vec<String>,
  pub begin: Vec<String>,
  pub give_up: Vec<String>,
  pub stop: Vec<String>,
  /// What may come before an offered opening word, besides the begin words.
  pub offer: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Times {
  pub recruit_seconds: u64,
  pub idle_seconds: u64,
  pub robot_ms: u64,
  pub least_turn_seconds: u64,
  pub most_turn_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rounds {
  /// How many words a round's opening word must leave to answer with.
  pub answers: u64,
  /// How many candidates the robot weighs before it moves.
  pub robot_reach: usize,
  /// The syllables a round's word is drawn from.
  pub syllables: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Points {
  pub per_syllable: i64,
  pub hanbang: i64,
  pub survivor: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Names {
  /// How many players a game's name lists before it counts the rest.
  pub listed: usize,
}
