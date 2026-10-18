//! The Hangul arithmetic, held to the cases of `tests/fixtures/hangul.toml`: the initial-sound rule turning one
//! way only, a word being whole syllables, and a word's last final consonant.

use serde::Deserialize;
use word_chain::hangul::{is_hangul_word, last_final, openings_for};

#[derive(Deserialize)]
struct Cases {
  openings: Vec<Openings>,
  words: Words,
  finals: Vec<Final>,
}

#[derive(Deserialize)]
struct Openings {
  syllable: char,
  openings: Vec<String>,
}

#[derive(Deserialize)]
struct Words {
  whole: Vec<String>,
  not_whole: Vec<String>,
}

#[derive(Deserialize)]
struct Final {
  word: String,
  last: u32,
}

fn cases() -> Cases {
  toml::from_str(include_str!("fixtures/hangul.toml")).expect("the Hangul fixture reads")
}

#[test]
fn the_initial_sound_rule_turns_rieul_and_nieun_one_way() {
  for case in cases().openings {
    assert_eq!(openings_for(case.syllable), case.openings, "{}", case.syllable);
  }
}

#[test]
fn a_word_is_whole_syllables_and_nothing_else() {
  let cases = cases();

  for word in &cases.words.whole {
    assert!(is_hangul_word(word), "{word}");
  }

  for word in &cases.words.not_whole {
    assert!(!is_hangul_word(word), "{word:?}");
  }

  for case in &cases.finals {
    assert_eq!(last_final(&case.word), case.last, "{}", case.word);
  }
}
