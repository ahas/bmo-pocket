//! The word book (docs/word-chain.md, 「The word book」): installed out of KKuTu's word list and the NIKL
//! dictionaries, and a word looked up in it. Each answers in English lines, and refuses with why.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use rusqlite::{Connection, params};

use crate::book::{ChainRules, SqliteWordBook, WordBook, openings_after};
use crate::hangul::is_hangul_word;
use crate::policy::WORD_CHAIN;

// How long a fetched part of the sources is waited on.
const FETCH_WAIT: Duration = Duration::from_secs(120);

/// Where the installed word book stands under a data root.
pub fn book_path(data_root: &Path) -> PathBuf {
  data_root.join("word-chain").join("ko.sqlite")
}

/// A word looked up in the book: whether it stands and is a loanword, how many words follow it, and its
/// meaning where the book holds one.
pub fn look_up(path: &Path, word: &str) -> Result<Vec<String>, String> {
  let book = SqliteWordBook::open(path)
    .map_err(|why| format!("the word book at {} cannot be opened: {why}", path.display()))?;
  let Some(entry) = book.find(word) else {
    return Ok(vec![format!("{word}: not in the book")]);
  };
  let next = openings_after(word);
  let rules = ChainRules {
    loanwords: true,
    manner: false,
  };
  let loanword = if entry.loanword { ", a loanword" } else { "" };
  let mut lines = vec![format!(
    "{word}: in the book{loanword}; {} words follow it, starting with {}",
    book.followers(&next, rules),
    next.join("/")
  )];

  if let Some(meaning) = book.meaning(word) {
    let kind = if meaning.basic { "a basic word" } else { "a harder word" };

    lines.push(format!("{kind}: {}", meaning.text));
  }

  Ok(lines)
}

/// One playable word of KKuTu's list and whether it is a loanword.
pub type Entry = (String, bool);

// The word flags the dump carries, as bits: the loanword one, and anything above it — an accepted
// non-dictionary word, one written with spaces, a dialect word, an old form or a North Korean standard
// word — which is no plain dictionary noun.
const LOANWORD: i64 = 1;
const NOT_PLAIN: i64 = !LOANWORD;
// The part-of-speech code the dump gives a noun.
const NOUN: &str = "1";

/// The playable words of KKuTu's PostgreSQL dump: nouns of two syllables or more, spelled in Hangul alone,
/// with no flag but the loanword one. Only the facts a chain game judges by are taken; the dump's meanings
/// are placeholders.
///
/// The words stand in the dump as the text of one `COPY kkutu_ko` block, a row to a line and its columns
/// split by tabs, which is PostgreSQL's own dump text and no format a library reads; the block's own header
/// names its columns.
pub fn kkutu_words(dump: &str) -> Result<Vec<Entry>, String> {
  const HEADER: &str = "COPY kkutu_ko (";

  let start = dump.find(HEADER).ok_or("the dump has no kkutu_ko table")?;
  let named = &dump[start + HEADER.len()..];
  let columns: Vec<String> = named[..named.find(')').unwrap_or(0)]
    .split(',')
    .map(|column| column.trim().replace('"', ""))
    .collect();
  let at = |name: &str| {
    columns
      .iter()
      .position(|column| column == name)
      .ok_or_else(|| format!("the kkutu_ko table has no {name} column"))
  };
  let (word_at, type_at, flag_at) = (at("_id")?, at("type")?, at("flag")?);
  let mut lines = dump[start..].lines().skip(1);
  let mut words = Vec::new();

  for line in lines.by_ref() {
    if line == "\\." {
      return Ok(words);
    }

    let fields: Vec<&str> = line.split('\t').collect();
    let word = fields.get(word_at).copied().unwrap_or_default();
    let flag: Option<i64> = fields.get(flag_at).and_then(|flag| flag.trim().parse().ok());
    let noun = fields.get(type_at).is_some_and(|kind| kind.split(',').any(|part| part == NOUN));

    match flag {
      Some(flag) if noun && flag & NOT_PLAIN == 0 && word.chars().count() >= 2 && is_hangul_word(word) => {
        words.push((word.to_owned(), flag & LOANWORD != 0));
      }
      _ => {}
    }
  }

  Err("the kkutu_ko table does not end".to_owned())
}

fn raw_base() -> String {
  let nikl = &WORD_CHAIN.install.nikl;

  format!("https://raw.githubusercontent.com/spellcheck-ko/korean-dict-nikl/{}", nikl.commit)
}

/// The standard dictionary's parts at the pinned commit: one every five thousand entries, and the rest.
pub fn stdict_urls() -> Vec<String> {
  let nikl = &WORD_CHAIN.install.nikl;

  (1..=nikl.stdict_parts)
    .map(|part| format!("{:06}", part * 5000))
    .chain(std::iter::once(nikl.stdict_last.clone()))
    .map(|part| format!("{}/stdict/{part}.xml", raw_base()))
    .collect()
}

/// The basic dictionary's parts at the pinned commit.
pub fn krdict_urls() -> Vec<String> {
  (1..=WORD_CHAIN.install.nikl.krdict_parts)
    .map(|part| format!("{}/krdict/{part:03}.xml", raw_base()))
    .collect()
}

fn element<'a, 'input>(
  node: roxmltree::Node<'a, 'input>,
  name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
  node.descendants().find(|child| child.has_tag_name(name))
}

fn text_of(node: Option<roxmltree::Node<'_, '_>>) -> String {
  node
    .map(|node| {
      node
        .descendants()
        .filter(roxmltree::Node::is_text)
        .filter_map(|child| child.text())
        .collect::<String>()
    })
    .unwrap_or_default()
}

fn headword(text: &str) -> String {
  text
    .chars()
    .filter(|character| *character != '-' && *character != '^' && !character.is_whitespace())
    .collect()
}

fn parsed(xml: &str) -> Result<roxmltree::Document<'_>, String> {
  roxmltree::Document::parse_with_options(
    xml,
    roxmltree::ParsingOptions {
      allow_dtd: true,
      ..Default::default()
    },
  )
  .map_err(|why| format!("a part of the NIKL dictionaries does not read: {why}"))
}

/// Each noun of a part of the standard dictionary with its homograph number — 0 for a word with none — and
/// its first plain sense, its field before it.
pub fn stdict_meanings(xml: &str) -> Result<Vec<(String, u32, String)>, String> {
  let nikl = &WORD_CHAIN.install.nikl;
  let document = parsed(xml)?;
  let mut found = Vec::new();

  for item in document.descendants().filter(|node| node.has_tag_name("item")) {
    let written = item
      .children()
      .find(|child| child.has_tag_name("word_info"))
      .and_then(|info| info.children().find(|child| child.has_tag_name("word")));
    // A homograph carries its number after the word: a headword ending in 03 is the word's third.
    let head = headword(&text_of(written));
    let word = head.trim_end_matches(|character: char| character.is_ascii_digit()).to_owned();
    let homograph: u32 = head[word.len()..].parse().unwrap_or(0);

    if word.is_empty() {
      continue;
    }

    for pos in item.descendants().filter(|node| node.has_tag_name("pos_info")) {
      if text_of(element(pos, "pos")) != nikl.noun {
        continue;
      }

      let meaning = pos
        .descendants()
        .filter(|node| node.has_tag_name("sense_info"))
        .filter(|sense| !nikl.off_standard.contains(&text_of(element(*sense, "type"))))
        .find_map(sense_line);

      if let Some(meaning) = meaning {
        found.push((word.clone(), homograph, meaning));
        break;
      }
    }
  }

  Ok(found)
}

// The definition's own small markup, which the dictionary writes as text inside it: a cross-reference's
// number in its own tags, any other tag, and a homograph's number after a word it quotes, which reads as the
// word alone.
static NUMBERS: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"<sp_no>[^<]*</sp_no>|<each_sense_no>[^<]*</each_sense_no>").expect("a pattern")
});
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"</?[a-z_]+(?:\s[^>]*)?>").expect("a pattern"));
static HOMOGRAPH: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"(‘[^’]*?)\d{2,3}(’)").expect("a pattern"));

// One sense as a line: its field, then its definition with the dictionary's markup taken out and cut to a
// line; none where it only points elsewhere or names one person, place or book.
fn sense_line(sense: roxmltree::Node<'_, '_>) -> Option<String> {
  let nikl = &WORD_CHAIN.install.nikl;
  let definition = element(sense, "definition")
    .map(|definition| {
      definition
        .descendants()
        .filter(|node| node.is_text())
        .filter(|node| {
          !node
            .ancestors()
            .any(|above| above.has_tag_name("sp_no") || above.has_tag_name("each_sense_no"))
        })
        .filter_map(|node| node.text())
        .collect::<String>()
    })
    .unwrap_or_default();
  let definition = NUMBERS.replace_all(&definition, "");
  let definition = TAGS.replace_all(&definition, "");
  let definition = HOMOGRAPH.replace_all(&definition, "$1$2");
  let definition = definition.split_whitespace().collect::<Vec<_>>().join(" ");

  if definition.is_empty() || definition.starts_with('→') {
    return None;
  }

  let field = text_of(element(sense, "cat")).trim().to_owned();

  if nikl.proper_fields.contains(&field) {
    return None;
  }

  // The first sentence alone where the whole runs past a line.
  let first = if definition.chars().count() <= nikl.meaning_chars {
    definition.clone()
  } else {
    first_sentence(&definition).unwrap_or(&definition).to_owned()
  };
  let line = if field.is_empty() || field == nikl.no_field {
    first
  } else {
    format!("({field}) {first}")
  };

  Some(if line.chars().count() <= nikl.meaning_chars {
    line
  } else {
    format!("{}…", line.chars().take(nikl.meaning_chars - 1).collect::<String>().trim_end())
  })
}

/// A text's first sentence: up to the first full stop that a space or the end follows.
pub fn first_sentence(text: &str) -> Option<&str> {
  text.char_indices().find_map(|(at, character)| {
    let after = &text[at + character.len_utf8()..];

    (character == '.' && after.chars().next().is_none_or(char::is_whitespace)).then(|| &text[..at + 1])
  })
}

/// Every headword of a part of the basic dictionary.
pub fn krdict_words(xml: &str) -> Result<Vec<String>, String> {
  let document = parsed(xml)?;

  Ok(
    document
      .descendants()
      .filter(|node| node.has_tag_name("Lemma"))
      .filter_map(|lemma| {
        lemma
          .children()
          .find(roxmltree::Node::is_element)
          .filter(|feat| feat.has_tag_name("feat") && feat.attribute("att") == Some("writtenForm"))
          .and_then(|feat| feat.attribute("val"))
          .map(headword)
      })
      .filter(|word| !word.is_empty())
      .collect(),
  )
}

// One part of the sources: read from the folder named, by the part's own folder and name, or fetched.
async fn part(url: &str, from: Option<&Path>, what: &str) -> Result<String, String> {
  if let Some(from) = from {
    let path = Path::new(url);
    let folder = path.parent().and_then(Path::file_name).unwrap_or_default();
    let name = path.file_name().unwrap_or_default();

    return std::fs::read_to_string(from.join(folder).join(name))
      .map_err(|why| format!("{}: {why}", from.display()));
  }

  let answer = reqwest::Client::new()
    .get(url)
    .timeout(FETCH_WAIT)
    .send()
    .await
    .map_err(|why| format!("{what} did not come: {}", why.without_url()))?;

  if !answer.status().is_success() {
    return Err(format!("{what} did not come: HTTP {} from {url}", answer.status().as_u16()));
  }

  answer.text().await.map_err(|why| format!("{what} did not come: {}", why.without_url()))
}

/// The word book installed: KKuTu's playable words with whether each is a loanword, and one meaning to a noun
/// of the standard dictionary — its first homograph's — marked basic where the basic dictionary lists the
/// word. It is written beside the book and put in its place whole.
pub async fn install(
  path: &Path,
  from: Option<&Path>,
  nikl_from: Option<&Path>,
) -> Result<Vec<String>, String> {
  let install = &WORD_CHAIN.install;
  let kkutu_url =
    format!("https://raw.githubusercontent.com/JJoriping/KKuTu/{}/db.sql", install.kkutu.commit);
  let dump = match from {
    Some(from) => std::fs::read_to_string(from).map_err(|why| format!("{}: {why}", from.display()))?,
    None => part(&kkutu_url, None, "KKuTu's word dump").await?,
  };
  let words = kkutu_words(&dump)?;
  let mut basic = HashSet::new();

  for url in krdict_urls() {
    basic.extend(krdict_words(&part(&url, nikl_from, "a part of the NIKL dictionaries").await?)?);
  }

  let mut first: HashMap<String, (u32, String)> = HashMap::new();
  let mut order: Vec<String> = Vec::new();

  for url in stdict_urls() {
    for (word, homograph, meaning) in
      stdict_meanings(&part(&url, nikl_from, "a part of the NIKL dictionaries").await?)?
    {
      match first.get(&word) {
        Some((standing, _)) if *standing <= homograph => {}
        Some(_) => {
          first.insert(word, (homograph, meaning));
        }
        None => {
          order.push(word.clone());
          first.insert(word, (homograph, meaning));
        }
      }
    }
  }

  let meanings: Vec<(String, String, bool)> = order
    .into_iter()
    .filter_map(|word| first.remove(&word).map(|(_, meaning)| (word, meaning)))
    .map(|(word, meaning)| {
      let listed = basic.contains(&word);

      (word, meaning, listed)
    })
    .collect();
  let partial =
    path.with_file_name(format!("{}.partial", path.file_name().unwrap_or_default().to_string_lossy()));

  if let Some(folder) = path.parent() {
    std::fs::create_dir_all(folder).map_err(|why| why.to_string())?;
  }

  let _ = std::fs::remove_file(&partial);

  let (count, kept) = write_book(&partial, &words, &meanings)
    .map_err(|why| format!("the word book could not be written: {why}"))?;

  std::fs::rename(&partial, path).map_err(|why| why.to_string())?;

  let short = |commit: &str| commit.chars().take(8).collect::<String>();

  Ok(vec![
    format!(
      "installed {count} words at {} ({} @ {}, {})",
      path.display(),
      install.kkutu.repository,
      short(&install.kkutu.commit),
      install.kkutu.licence
    ),
    format!(
      "added {kept} meanings ({} @ {}, {})",
      install.nikl.repository,
      short(&install.nikl.commit),
      install.nikl.licence
    ),
  ])
}

/// The book written: its words, the openers they make, the meanings of the words it holds, and what it was
/// read out of; how many words and meanings it holds.
pub fn write_book(
  path: &Path,
  words: &[Entry],
  meanings: &[(String, String, bool)],
) -> rusqlite::Result<(usize, usize)> {
  let install = &WORD_CHAIN.install;
  let mut db = Connection::open(path)?;
  let tx = db.transaction()?;

  tx.execute_batch(
    "CREATE TABLE IF NOT EXISTS words (word TEXT PRIMARY KEY, first TEXT NOT NULL, last TEXT NOT NULL, loanword INTEGER NOT NULL) WITHOUT ROWID;
     CREATE INDEX IF NOT EXISTS words_first ON words (first, loanword);
     CREATE TABLE IF NOT EXISTS openers (first TEXT PRIMARY KEY, total INTEGER NOT NULL, native INTEGER NOT NULL) WITHOUT ROWID;
     CREATE TABLE IF NOT EXISTS meanings (word TEXT PRIMARY KEY, text TEXT NOT NULL, basic INTEGER NOT NULL) WITHOUT ROWID;
     CREATE TABLE IF NOT EXISTS about (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
     DELETE FROM words; DELETE FROM openers; DELETE FROM meanings; DELETE FROM about;",
  )?;

  for (word, loanword) in words {
    let first = word.chars().next().map(String::from).unwrap_or_default();
    let last = word.chars().last().map(String::from).unwrap_or_default();

    tx.execute(
      "INSERT OR REPLACE INTO words (word, first, last, loanword) VALUES (?1, ?2, ?3, ?4)",
      params![word, first, last, i64::from(*loanword)],
    )?;
  }

  tx.execute("INSERT INTO openers (first, total, native) SELECT first, count(*), sum(loanword = 0) FROM words GROUP BY first", [])?;

  let mut kept = 0;

  for (word, text, basic) in meanings {
    kept += tx.execute(
      "INSERT OR IGNORE INTO meanings (word, text, basic) SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM words WHERE word = ?1)",
      params![word, text, i64::from(*basic)],
    )?;
  }

  let installed = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
  let about = [
    ("source", install.kkutu.repository.clone()),
    ("commit", install.kkutu.commit.clone()),
    ("licence", install.kkutu.licence.clone()),
    ("installed", installed),
    ("words", words.len().to_string()),
    ("meaningsSource", install.nikl.repository.clone()),
    ("meaningsCommit", install.nikl.commit.clone()),
    ("meaningsLicence", install.nikl.licence.clone()),
    ("meanings", kept.to_string()),
  ];

  for (key, value) in about {
    tx.execute("INSERT OR REPLACE INTO about (key, value) VALUES (?1, ?2)", params![key, value])?;
  }

  tx.commit()?;

  Ok((words.len(), kept))
}
