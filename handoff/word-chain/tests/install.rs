//! The word book (docs/word-chain.md, 「The word book」): its sources read, and the book installed from sources
//! on the disk and a word looked up in it through the binary. The sources' text is Korean, kept as the files of
//! `tests/fixtures/`.

use std::path::{Path, PathBuf};
use std::process::Command;

use word_chain::SqliteWordBook;
use word_chain::WordBook;
use word_chain::install::{
  first_sentence, kkutu_words, krdict_urls, krdict_words, stdict_meanings, stdict_urls, write_book,
};

fn fixture(name: &str) -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

fn read(name: &str) -> String {
  std::fs::read_to_string(fixture(name)).unwrap()
}

#[test]
fn reads_the_plain_nouns_of_the_kkutu_ko_table_and_marks_the_loanwords() {
  let expected: Vec<(String, bool)> = read("kkutu.expected")
    .lines()
    .map(|line| {
      let (word, loanword) = line.split_once('\t').unwrap();

      (word.to_owned(), loanword == "1")
    })
    .collect();

  assert_eq!(kkutu_words(&read("kkutu.sql")).unwrap(), expected);
}

#[test]
fn refuses_a_dump_without_the_table() {
  assert!(
    kkutu_words("COPY kkutu_en (_id) FROM stdin;\n\\.\n")
      .unwrap_err()
      .contains("no kkutu_ko table")
  );
}

#[test]
fn reads_each_nouns_first_plain_sense_its_field_first_and_the_markup_out_cut_to_a_line() {
  let expected: Vec<(String, u32, String)> = read("stdict.expected")
    .lines()
    .map(|line| {
      let mut fields = line.splitn(3, '\t');

      (
        fields.next().unwrap().to_owned(),
        fields.next().unwrap().parse().unwrap(),
        fields.next().unwrap().to_owned(),
      )
    })
    .collect();

  assert_eq!(stdict_meanings(&read("stdict.xml")).unwrap(), expected);
}

#[test]
fn reads_the_basic_dictionarys_headwords() {
  assert_eq!(
    krdict_words(&read("krdict.xml")).unwrap(),
    read("krdict.expected").lines().collect::<Vec<_>>()
  );
}

// The sources on the disk: KKuTu's dump, and a mirror of the NIKL parts with one noun's meaning and the basic
// dictionary listing it, every other part empty.
fn sources(root: &Path) -> (String, String) {
  let nikl = root.join("nikl");

  for url in stdict_urls().iter().chain(krdict_urls().iter()) {
    let path = Path::new(url);
    let folder = nikl.join(path.parent().unwrap().file_name().unwrap());

    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(path.file_name().unwrap()), "<channel/>").unwrap();
  }

  std::fs::copy(
    fixture("book/stdict.xml"),
    nikl.join("stdict").join(Path::new(&stdict_urls()[0]).file_name().unwrap()),
  )
  .unwrap();
  std::fs::copy(
    fixture("book/krdict.xml"),
    nikl.join("krdict").join(Path::new(&krdict_urls()[0]).file_name().unwrap()),
  )
  .unwrap();

  (
    fixture("book/db.sql").to_string_lossy().into_owned(),
    nikl.to_string_lossy().into_owned(),
  )
}

fn word_chain(data: &Path, args: &[&str]) -> (i32, String, String) {
  let done = Command::new(env!("CARGO_BIN_EXE_bmo-word-chain"))
    .arg("--data-root")
    .arg(data)
    .args(args)
    .output()
    .unwrap();

  (
    done.status.code().unwrap_or(-1),
    String::from_utf8_lossy(&done.stdout).into_owned(),
    String::from_utf8_lossy(&done.stderr).into_owned(),
  )
}

#[test]
fn the_book_is_installed_from_its_sources_and_a_word_is_looked_up_in_it() {
  let root = tempfile::tempdir().unwrap();
  let data = root.path().join("data");
  let (dump, nikl) = sources(root.path());
  let lookups = read("book/lookups.txt");
  let mut lookups = lookups.lines();
  let (present, absent) = (lookups.next().unwrap(), lookups.next().unwrap());
  let (code, out, err) = word_chain(&data, &["install", "--from", &dump, "--nikl-from", &nikl]);

  assert_eq!((code, err.as_str()), (0, ""), "{out}");
  assert_eq!(out.lines().count(), 2);
  assert!(out.contains("3"), "{out}");
  assert!(data.join("word-chain").join("ko.sqlite").exists());
  assert!(!data.join("word-chain").join("ko.sqlite.partial").exists());

  let (code, found, _) = word_chain(&data, &["word", present]);

  assert_eq!(code, 0);
  assert_eq!(found.lines().count(), 2, "{found}");
  assert!(found.lines().nth(1).unwrap().ends_with("A train."), "{found}");

  let (_, missing, _) = word_chain(&data, &["word", absent]);

  assert_eq!(missing.lines().count(), 1, "{missing}");
}

#[test]
fn keeps_the_first_meaning_of_a_word_the_book_holds() {
  let folder = tempfile::tempdir().unwrap();
  let path = folder.path().join("ko.sqlite");
  let meanings = [
    ("pus".to_owned(), "a yellow fluid.".to_owned(), false),
    ("pus".to_owned(), "a tie of a coat.".to_owned(), false),
    ("samarium".to_owned(), "a rare earth.".to_owned(), false),
  ];

  assert_eq!(write_book(&path, &[("pus".to_owned(), false)], &meanings).unwrap(), (1, 1));
  assert_eq!(SqliteWordBook::open(&path).unwrap().meaning("pus").unwrap().text, "a yellow fluid.");
}

#[test]
fn a_long_definition_keeps_its_first_sentence() {
  assert_eq!(first_sentence("one. two."), Some("one."));
  assert_eq!(first_sentence("v1.5 is out. next"), Some("v1.5 is out."));
  assert_eq!(first_sentence("no stop"), None);
}
