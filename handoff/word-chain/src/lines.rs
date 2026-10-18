//! What each thing a game says is, as a code with the facts it reads (docs/word-chain.md, 「How a game speaks」):
//! no sentence stands here. A room's words are rendered from the code and its fields by the web unit's language
//! pack (`word-chain.<code>`), which also says how a word, a syllable and a name are marked and which particle
//! follows each.

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::book::{ChainRules, WordBook, openings_after};
use crate::game::{Difficulty, Elimination, Ending, GameEvent, Player, Rejection};
use crate::hangul::is_hangul_word;
use crate::policy::WORD_CHAIN;

/// One line a game says: its code and the facts it reads.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Line {
  pub code: &'static str,
  pub fields: Map<String, Value>,
}

impl Line {
  /// A line of this code reading these fields, an object.
  pub fn of(code: &'static str, fields: Value) -> Self {
    let fields = match fields {
      Value::Object(fields) => fields,
      _ => Map::new(),
    };

    Self { code, fields }
  }

  fn bare(code: &'static str) -> Self {
    Self::of(code, json!({}))
  }
}

/// One message into a room: every line one step of a game said, in order, its game's name first where another
/// game stood beside it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Message {
  pub chat_id: i64,
  pub lines: Vec<Line>,
}

/// The names a game goes by: its people, or the robot's where nobody else is in it.
pub fn names_of(people: &[(String, String)], robot_name: &str) -> Vec<String> {
  if people.is_empty() {
    return vec![robot_name.to_owned()];
  }

  people.iter().map(|(_, name)| name.clone()).collect()
}

/// A game's name: its people's names, of which the first `listed` are shown and the rest counted.
pub fn game_line(people: &[(String, String)], robot_name: &str) -> Line {
  Line::of(
    "game",
    json!({ "names": names_of(people, robot_name), "listed": WORD_CHAIN.names.listed }),
  )
}

/// Why a word the asker gave cannot open the game, as `{reason, word}`: `not-in-book`, `loanword` or
/// `no-follower`; nothing where it can.
pub fn opening_refusal(book: &dyn WordBook, word: &str, rules: ChainRules) -> Option<Value> {
  let entry = if word.chars().count() < 2 || !is_hangul_word(word) { None } else { book.find(word) };
  let unusable = |reason: &str| Some(json!({ "reason": reason, "word": word }));
  let Some(entry) = entry else {
    return unusable("not-in-book");
  };

  if entry.loanword && !rules.loanwords {
    return unusable("loanword");
  }

  let used = std::iter::once(word.to_owned()).collect();

  if book.has_follower(&openings_after(word), rules, &used) {
    None
  } else {
    unusable("no-follower")
  }
}

/// The asker asked for another opening word in the room.
pub fn ask_again(unusable: &Value, name: &str) -> Line {
  Line::of("ask-again", json!({ "unusable": unusable, "name": name }))
}

/// The asker told again the word cannot open the game.
pub fn ask_again_bare(unusable: &Value) -> Line {
  Line::of("ask-again-bare", json!({ "unusable": unusable }))
}

/// The asker asked for the opening word first, a bare begin word having come.
pub fn ask_first(name: &str) -> Line {
  Line::of("ask-first", json!({ "name": name }))
}

/// A game folded for want of a usable opening word.
pub fn no_opening() -> Line {
  Line::bare("no-opening")
}

/// A game folded before it opened.
pub fn folded() -> Line {
  Line::bare("folded")
}

/// Several games a word could be for: which to pick, `join` or `stop`, the games by their people's names, and a
/// name to give as an example.
pub fn pick(act: &str, games: &[Vec<String>], example: &str) -> Line {
  Line::of("pick", json!({ "for": act, "games": games, "example": example }))
}

fn difficulty(difficulty: Difficulty) -> &'static str {
  match difficulty {
    Difficulty::Easy => "easy",
    Difficulty::Normal => "normal",
    Difficulty::Hard => "hard",
  }
}

fn idle_minutes() -> u64 {
  (WORD_CHAIN.times.idle_seconds + 30) / 60
}

/// What a call for players says of the game it opens.
pub struct Call<'a> {
  pub robot: Option<Difficulty>,
  pub robot_name: &'a str,
  pub loanwords: bool,
  pub manner: bool,
  pub seconds: Option<u64>,
  pub wait_ms: u64,
  pub opening: Option<&'a str>,
  /// A name of this game's own to join it by, where another game recruits beside it.
  pub join_name: Option<String>,
}

/// The call for players: whether the robot plays and how hard, the rules, the time a move has or the minutes a
/// game waits on none, the seconds until it starts, the word it opens on, and the name to join it by.
pub fn opening_line(call: &Call<'_>) -> Line {
  Line::of(
    "call",
    json!({
      "robot": call.robot.map(difficulty),
      "robot_name": call.robot_name,
      "loanwords": call.loanwords,
      "manner": call.manner,
      "seconds": call.seconds,
      "idle_minutes": idle_minutes(),
      "wait_seconds": (call.wait_ms + 500) / 1000,
      "opening": call.opening,
      "join_name": call.join_name,
    }),
  )
}

fn rejection_line(reason: Rejection, word: &str, openings: &[String]) -> Line {
  match reason {
    Rejection::OffChain => Line::of("off-chain", json!({ "word": word, "openings": openings })),
    Rejection::NotInBook => Line::of("not-in-book", json!({ "word": word })),
    Rejection::Loanword => Line::of("loanword", json!({ "word": word })),
    Rejection::Used => Line::of("used", json!({ "word": word })),
    Rejection::Manner => Line::of("manner", json!({ "word": word })),
  }
}

// A line giving what a word means, where it is not a basic word and the book knows its meaning.
fn meaning_line(book: &dyn WordBook, word: &str) -> Option<Line> {
  book
    .meaning(word)
    .filter(|meaning| !meaning.basic)
    .map(|meaning| Line::of("meaning", json!({ "word": word, "meaning": meaning.text })))
}

/// What one event says, its players as they stood when it happened.
pub fn event_lines(event: &GameEvent, players: &[Player], book: &dyn WordBook) -> Vec<Line> {
  match event {
    GameEvent::Joined { player } => {
      vec![Line::of("joined", json!({ "name": player.name, "count": players.len() }))]
    }
    GameEvent::Round { word, first, .. } => {
      let mut said = if *first {
        let order: Vec<&str> = players.iter().map(|player| player.name.as_str()).collect();

        vec![
          Line::of("started", json!({ "order": order })),
          Line::of("first-word", json!({ "word": word })),
        ]
      } else {
        vec![Line::of("new-word", json!({ "word": word }))]
      };

      said.extend(meaning_line(book, word));
      said
    }
    GameEvent::Turn {
      player,
      openings,
      seconds,
    } => vec![Line::of(
      "turn",
      json!({ "name": player.name, "openings": openings, "seconds": seconds }),
    )],
    GameEvent::Accepted {
      player,
      word,
      points,
      ..
    } => {
      let taken = if player.robot {
        Line::of("taken-robot", json!({ "name": player.name, "word": word, "points": points }))
      } else {
        Line::of("taken", json!({ "word": word, "points": points }))
      };

      std::iter::once(taken).chain(meaning_line(book, word)).collect()
    }
    GameEvent::Rejected {
      word,
      reason,
      openings,
      ..
    } => vec![rejection_line(*reason, word, openings)],
    GameEvent::Eliminated {
      player,
      reason,
      openings,
      by,
    } => vec![match reason {
      Elimination::Timeout => Line::of("timeout", json!({ "name": player.name })),
      Elimination::Hanbang => Line::of(
        "hanbang",
        json!({
          "openings": openings,
          "name": player.name,
          "by": by.as_ref().map(|by| json!({ "name": by.name, "points": WORD_CHAIN.points.hanbang })),
        }),
      ),
      Elimination::GaveUp => Line::of("gave-up", json!({ "name": player.name })),
      Elimination::Stuck => Line::of("stuck", json!({ "name": player.name })),
    }],
    GameEvent::Over { ranking, reason } => over_lines(ranking, *reason),
  }
}

fn over_lines(ranking: &[Player], reason: Ending) -> Vec<Line> {
  let head = match reason {
    Ending::Nobody => return vec![Line::bare("nobody")],
    Ending::Stopped => Line::bare("stopped"),
    Ending::Idle => Line::of("idle", json!({ "minutes": idle_minutes() })),
    Ending::Decided => Line::bare("decided"),
  };
  let rows = ranking.iter().enumerate().map(|(at, player)| {
    Line::of(
      "ranking",
      json!({ "place": at + 1, "name": player.name, "score": player.score, "words": player.words }),
    )
  });

  std::iter::once(head).chain(rows).collect()
}
