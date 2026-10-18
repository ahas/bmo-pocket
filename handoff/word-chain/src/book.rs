//! The word book a game judges by and its robot plays from: the installed SQLite file the word chain's
//! install verb writes (docs/word-chain.md, 「The word book」), read and never written here.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use rand::seq::SliceRandom;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::hangul::openings_for;

/// What a game allows beyond the plain noun.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainRules {
  /// Loanwords are playable.
  pub loanwords: bool,
  /// A word whose last syllable nothing follows (a hanbang word) is refused rather than played.
  pub manner: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordEntry {
  pub word: String,
  pub loanword: bool,
}

/// What a word means, and whether it is a basic word — one the dictionary for learners lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordMeaning {
  pub text: String,
  pub basic: bool,
}

/// The dictionary a game judges by and its robot plays from.
pub trait WordBook: Send + Sync {
  /// The entry, or nothing where the word is not in the book.
  fn find(&self, word: &str) -> Option<WordEntry>;
  /// The word's meaning, or nothing where the book has none for it.
  fn meaning(&self, word: &str) -> Option<WordMeaning>;
  /// Up to `limit` words the rules allow, starting with one of the openings and not yet used, in
  /// random order.
  fn sample(
    &self,
    openings: &[String],
    rules: ChainRules,
    used: &HashSet<String>,
    limit: usize,
  ) -> Vec<String>;
  /// How many words the rules allow that start with one of the openings, used ones included.
  fn followers(&self, openings: &[String], rules: ChainRules) -> u64;
  /// Whether any word the rules allow and not yet used starts with one of the openings.
  fn has_follower(&self, openings: &[String], rules: ChainRules, used: &HashSet<String>) -> bool;
}

/// The openings a word hands the next player: its last syllable and that syllable's initial-sound
/// variant.
pub fn openings_after(word: &str) -> Vec<String> {
  word.chars().last().map(openings_for).unwrap_or_default()
}

/// A word book over a SQLite file, one connection asked by one game at a time.
pub struct SqliteWordBook {
  db: Mutex<Connection>,
}

// What a statement the book cannot read comes to: said once, and read as an empty answer, so a damaged
// book refuses words rather than ending the game.
fn read<T>(asked: &str, answer: rusqlite::Result<T>, otherwise: T) -> T {
  answer.unwrap_or_else(|error| {
    eprintln!("word chain: the word book did not answer {asked}: {error}");
    otherwise
  })
}

impl SqliteWordBook {
  /// The installed book at `path`, opened to be read.
  pub fn open(path: &Path) -> rusqlite::Result<Self> {
    let db = Connection::open_with_flags(
      path,
      OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    Ok(Self::over(db))
  }

  /// A book over a connection already open, such as one a check wrote with `write`.
  pub fn over(db: Connection) -> Self {
    Self { db: Mutex::new(db) }
  }

  fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
    self.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
  }

  /// A book written at `path` from a list of words, as `write` writes one.
  pub fn write_file(
    path: &Path,
    words: &[(&str, bool)],
    meanings: &[(&str, &str, bool)],
  ) -> rusqlite::Result<()> {
    Self::write(&Connection::open(path)?, words, meanings)
  }

  /// A book written from a list of words and whether each is a loanword, with the meanings given for
  /// words it holds: the shape the install verb writes, for a check that needs a book.
  pub fn write(
    db: &Connection,
    words: &[(&str, bool)],
    meanings: &[(&str, &str, bool)],
  ) -> rusqlite::Result<()> {
    db.execute_batch(
      "CREATE TABLE IF NOT EXISTS words (word TEXT PRIMARY KEY, first TEXT NOT NULL, last TEXT NOT NULL, loanword INTEGER NOT NULL) WITHOUT ROWID;
       CREATE INDEX IF NOT EXISTS words_first ON words (first, loanword);
       CREATE TABLE IF NOT EXISTS openers (first TEXT PRIMARY KEY, total INTEGER NOT NULL, native INTEGER NOT NULL) WITHOUT ROWID;
       CREATE TABLE IF NOT EXISTS meanings (word TEXT PRIMARY KEY, text TEXT NOT NULL, basic INTEGER NOT NULL) WITHOUT ROWID;
       CREATE TABLE IF NOT EXISTS about (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;",
    )?;

    for (word, loanword) in words {
      let first = word.chars().next().map(String::from).unwrap_or_default();
      let last = word.chars().last().map(String::from).unwrap_or_default();

      db.execute(
        "INSERT OR REPLACE INTO words (word, first, last, loanword) VALUES (?1, ?2, ?3, ?4)",
        params![word, first, last, i64::from(*loanword)],
      )?;
    }

    db.execute(
      "INSERT INTO openers (first, total, native) SELECT first, count(*), sum(loanword = 0) FROM words GROUP BY first",
      [],
    )?;

    for (word, text, basic) in meanings {
      db.execute(
        "INSERT OR IGNORE INTO meanings (word, text, basic) SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM words WHERE word = ?1)",
        params![word, text, i64::from(*basic)],
      )?;
    }

    Ok(())
  }
}

impl WordBook for SqliteWordBook {
  fn find(&self, word: &str) -> Option<WordEntry> {
    let found = self
      .db()
      .query_row("SELECT loanword FROM words WHERE word = ?1", [word], |row| row.get::<_, i64>(0))
      .optional();

    read("a word", found, None).map(|loanword| WordEntry {
      word: word.to_owned(),
      loanword: loanword == 1,
    })
  }

  fn meaning(&self, word: &str) -> Option<WordMeaning> {
    let db = self.db();
    // A book installed before meanings were read has no such table.
    let held = db
      .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meanings'", [], |_| Ok(()))
      .optional();

    read("whether it holds meanings", held, None)?;

    let found = db
      .query_row("SELECT text, basic FROM meanings WHERE word = ?1", [word], |row| {
        Ok(WordMeaning {
          text: row.get(0)?,
          basic: row.get::<_, i64>(1)? == 1,
        })
      })
      .optional();

    read("a meaning", found, None)
  }

  fn sample(
    &self,
    openings: &[String],
    rules: ChainRules,
    used: &HashSet<String>,
    limit: usize,
  ) -> Vec<String> {
    let db = self.db();
    let reach = i64::try_from(limit + used.len()).unwrap_or(i64::MAX);
    let mut words = Vec::new();

    for opening in openings {
      let drawn = db
        .prepare_cached(
          "SELECT word FROM words WHERE first = ?1 AND loanword <= ?2 ORDER BY random() LIMIT ?3",
        )
        .and_then(|mut query| {
          query
            .query_map(params![opening, i64::from(rules.loanwords), reach], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
        });

      words.extend(read("a sample", drawn, Vec::new()));
    }

    words.retain(|word| !used.contains(word));
    words.shuffle(&mut rand::rng());
    words.truncate(limit);
    words
  }

  fn followers(&self, openings: &[String], rules: ChainRules) -> u64 {
    let db = self.db();

    openings
      .iter()
      .map(|opening| {
        let counted = db
          .query_row("SELECT total, native FROM openers WHERE first = ?1", [opening], |row| {
            Ok(if rules.loanwords { row.get::<_, i64>(0)? } else { row.get::<_, i64>(1)? })
          })
          .optional();

        u64::try_from(read("the openers", counted, None).unwrap_or(0)).unwrap_or(0)
      })
      .sum()
  }

  fn has_follower(&self, openings: &[String], rules: ChainRules, used: &HashSet<String>) -> bool {
    let db = self.db();
    // Only a used word can stand in the way, so reading one more row than there are used words settles
    // it without ordering the syllable's whole range.
    let reach = i64::try_from(used.len() + 1).unwrap_or(i64::MAX);

    openings.iter().any(|opening| {
      let read_words = db
        .prepare_cached("SELECT word FROM words WHERE first = ?1 AND loanword <= ?2 LIMIT ?3")
        .and_then(|mut query| {
          query
            .query_map(params![opening, i64::from(rules.loanwords), reach], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
        });

      read("the followers", read_words, Vec::new()).iter().any(|word| !used.contains(word))
    })
  }
}
