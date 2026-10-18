//! Hangul syllable arithmetic the word chain needs: taking a syllable apart, putting one together, and the
//! initial-sound rule by which a word ending in a syllable may be answered by one starting with its initial-sound
//! form. The rule's consonants and vowels are the policy's (`initial_sound`).

use crate::policy::WORD_CHAIN;

const SYLLABLE_FIRST: u32 = 0xac00;
const SYLLABLE_LAST: u32 = 0xd7a3;
const VOWELS: u32 = 21;
const FINALS: u32 = 28;

#[derive(Clone, Copy)]
struct Parts {
  initial: u32,
  vowel: u32,
  last: u32,
}

fn split(syllable: char) -> Option<Parts> {
  let code = u32::from(syllable);

  if !(SYLLABLE_FIRST..=SYLLABLE_LAST).contains(&code) {
    return None;
  }

  let offset = code - SYLLABLE_FIRST;

  Some(Parts {
    initial: offset / (VOWELS * FINALS),
    vowel: (offset / FINALS) % VOWELS,
    last: offset % FINALS,
  })
}

fn join(parts: Parts) -> char {
  char::from_u32(SYLLABLE_FIRST + (parts.initial * VOWELS + parts.vowel) * FINALS + parts.last)
    .unwrap_or('\u{fffd}')
}

/// Whether every character of the text is a complete Hangul syllable.
pub fn is_hangul_word(text: &str) -> bool {
  !text.is_empty() && text.chars().all(|ch| split(ch).is_some())
}

/// The syllable a word may start with in place of the given one under the initial-sound rule, or nothing where
/// the rule does not touch it. The rule runs one way only: a syllable turns into its initial-sound form, never
/// the form back into the syllable.
pub fn initial_sound_variant(syllable: char) -> Option<char> {
  let parts = split(syllable)?;
  let turn = WORD_CHAIN
    .initial_sound
    .iter()
    .find(|turn| turn.from == parts.initial && turn.vowels.contains(&parts.vowel))?;

  Some(join(Parts {
    initial: turn.to,
    ..parts
  }))
}

/// Every syllable a word may start with to follow a word ending in the given one: itself first.
pub fn openings_for(syllable: char) -> Vec<String> {
  std::iter::once(syllable)
    .chain(initial_sound_variant(syllable))
    .map(String::from)
    .collect()
}

/// The final consonant of a word's last syllable: 0 for none, 8 for rieul, and 0 for a word that does not end
/// in a Hangul syllable.
pub fn last_final(word: &str) -> u32 {
  word.chars().last().and_then(split).map_or(0, |parts| parts.last)
}
