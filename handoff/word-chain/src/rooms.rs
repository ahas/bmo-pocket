//! The games in the rooms (docs/word-chain.md, 「The games in the rooms」): the games standing in each room, how
//! a room's messages are read as moves while one stands, and what each step of a game says, one message a step.
//! The game itself is `game`'s; this side holds which room it is in, the words it is said in, and the clock it
//! keeps time on. What it says goes out as codes (`lines`), and what a start or a stop answers its caller is an
//! English sentence with a code of its own.
//!
//! Nothing here calls a model. A move is a lookup in the word book and a line sent into the room, so a game
//! costs a room nothing beyond the one turn that started it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::book::{ChainRules, SqliteWordBook, WordBook};
use crate::game::{Difficulty, GameEvent, GameSettings, Opening, Phase, RobotSeat, WordChainGame};
use crate::hangul::is_hangul_word;
use crate::lines::{
  Call, Line, Message, ask_again, ask_again_bare, ask_first, event_lines, folded, game_line, names_of,
  no_opening, opening_line, opening_refusal, pick,
};
use crate::policy::WORD_CHAIN;

/// The person who asked for a game, who is in it from the start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Starter {
  pub id: i64,
  pub name: String,
}

/// What a game is started with.
#[derive(Clone, Debug, PartialEq)]
pub struct WordChainStart {
  pub chat_id: i64,
  pub starter: Option<Starter>,
  /// Whether the robot plays, and how hard.
  pub robot: Option<Difficulty>,
  /// Seconds a player has for one move, or nothing where the room asked for no limit.
  pub turn_seconds: Option<f64>,
  pub loanwords: bool,
  pub manner: bool,
  /// The word the person who asked already gave to open with, or nothing where the book draws one.
  pub opening: Option<String>,
}

/// One message of a room, as far as a game reads it.
#[derive(Clone, Copy, Debug)]
pub struct RoomLine<'a> {
  pub chat_id: i64,
  pub sender_id: Option<i64>,
  pub sender_name: &'a str,
  pub text: &'a str,
  pub owner: bool,
}

/// What a start or a stop answers its caller: a code its caller decides on, and an English sentence saying it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Answer {
  pub code: &'static str,
  pub line: String,
}

impl Answer {
  fn of(code: &'static str, line: String) -> Self {
    Self { code, line }
  }
}

/// Sends one message into a room. It must not wait on the send.
pub type Say = Arc<dyn Fn(Message) + Send + Sync>;

/// Runs `fire` once `ms` have passed.
pub trait Clock: Send + Sync {
  fn after(&self, ms: u64, fire: Box<dyn FnOnce() + Send>);
}

/// The process's own clock: a task that sleeps and then fires.
pub struct TaskClock;

impl Clock for TaskClock {
  fn after(&self, ms: u64, fire: Box<dyn FnOnce() + Send>) {
    tokio::spawn(async move {
      tokio::time::sleep(Duration::from_millis(ms)).await;
      fire();
    });
  }
}

/// The lengths a game is held to where a check shortens them; the policy's otherwise.
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
  pub recruit_ms: u64,
  pub robot_ms: u64,
  pub round_answers: u64,
}

impl Default for Tuning {
  fn default() -> Self {
    Self {
      recruit_ms: WORD_CHAIN.times.recruit_seconds * 1000,
      robot_ms: WORD_CHAIN.times.robot_ms,
      round_answers: WORD_CHAIN.rounds.answers,
    }
  }
}

/// What the games stand on: where the word book is installed, the road a message goes out on, the clock, the
/// lengths, and the name the robot plays under, the room's agent's.
pub struct Parts {
  pub book_path: PathBuf,
  pub say: Say,
  pub clock: Arc<dyn Clock>,
  pub tuning: Tuning,
  pub robot_name: String,
}

/// The games standing in every room, shared by every hand that reaches them.
#[derive(Clone)]
pub struct WordChainRooms {
  inner: Arc<Inner>,
}

struct Inner {
  state: Mutex<State>,
  book_path: PathBuf,
  say: Say,
  clock: Arc<dyn Clock>,
  tuning: Tuning,
  robot_name: String,
}

// An alarm to set: the standing game it is for, what it does, and in how many milliseconds.
type Armed = (u64, Fire, u64);

// What an alarm a standing game set does when it comes due.
#[derive(Clone, Copy, Debug)]
enum Fire {
  Game(u64),
  Wait(u64),
}

struct Standing {
  id: u64,
  chat_id: i64,
  request: WordChainStart,
  /// The game, or nothing while it waits for a usable opening word from the person who asked.
  game: Option<WordChainGame>,
  /// The people in it, the robot aside, in the order they joined, by id and name: what the game is named by
  /// and whose words it hears.
  people: Vec<(String, String)>,
  /// The lines of the step being said, and whether its message opens on the game's name.
  pending: Vec<Line>,
  named: bool,
  /// The wait for an opening word, by its alarm.
  wait: Option<u64>,
}

impl Standing {
  fn has(&self, person: &str) -> bool {
    self.people.iter().any(|(id, _)| id == person)
  }

  fn recruiting(&self) -> bool {
    self.game.as_ref().is_some_and(|game| game.phase() == Phase::Recruiting)
  }

  fn rules(&self) -> ChainRules {
    ChainRules {
      loanwords: self.request.loanwords,
      manner: self.request.manner,
    }
  }
}

#[derive(Default)]
struct State {
  /// Every room's games, oldest first. A room holds any number; a person is in one.
  standings: Vec<Standing>,
  /// Games that ended in this step, kept until their last lines go out.
  closing: Vec<Standing>,
  /// The installed book, opened at the first need: nothing where none is installed.
  book: Option<Option<Arc<dyn WordBook>>>,
  next_id: u64,
  /// Messages said straight out, ahead of the step's batches.
  direct: Vec<Message>,
  /// The games with lines waiting, in the order their first line came.
  batches: Vec<u64>,
  /// Alarms to set once the step is over.
  alarms: Vec<Armed>,
}

// A word of a set said alone or followed by a name: the name, "" for none, or nothing where the message is not
// that word.
fn addressed(said: &str, words: &[String]) -> Option<String> {
  if words.iter().any(|word| word == said) {
    return Some(String::new());
  }

  words
    .iter()
    .find_map(|word| said.strip_prefix(word.as_str()).and_then(|rest| rest.strip_prefix(' ')))
    .map(|name| name.trim().to_owned())
}

// Whether a name given fits a player's: either starts the other, spaces, case and a word naming a game after it
// aside.
fn fits(given: &str, name: &str) -> bool {
  let squash = |text: &str| text.split_whitespace().collect::<String>().to_lowercase();
  let given = squash(given);
  let given = WORD_CHAIN
    .words
    .game
    .iter()
    .find_map(|word| given.strip_suffix(word.as_str()))
    .unwrap_or(&given);
  let name = squash(name);

  !given.is_empty() && !name.is_empty() && (name.starts_with(given) || given.starts_with(&name))
}

fn holds(words: &[String], said: &str) -> bool {
  words.iter().any(|word| word == said)
}

// What a pick between games does to the one picked.
enum Act {
  Join(String, String),
  End,
}

// A game's name as an answer says it: its people's names, the first `listed` and a count of the rest.
fn game_words(people: &[(String, String)], robot_name: &str) -> String {
  let names = names_of(people, robot_name);
  let listed = WORD_CHAIN.names.listed;
  let rest = names.len().saturating_sub(listed);
  let shown = names.iter().take(listed).cloned().collect::<Vec<_>>().join(", ");

  if rest > 0 { format!("[{shown} and {rest} more]") } else { format!("[{shown}]") }
}

// What an opening word that cannot open a game is, in an answer's words.
fn unusable_words(unusable: &Value) -> String {
  let word = unusable["word"].as_str().unwrap_or_default();
  let why = match unusable["reason"].as_str() {
    Some("loanword") => "is a loanword",
    Some("no-follower") => "leaves no word to follow it",
    _ => "is not in the word book",
  };

  format!("{word} {why}")
}

impl State {
  fn find(&mut self, id: u64) -> Option<&mut Standing> {
    self
      .standings
      .iter_mut()
      .chain(self.closing.iter_mut())
      .find(|standing| standing.id == id)
  }

  fn in_room(&self, chat_id: i64) -> Vec<u64> {
    self
      .standings
      .iter()
      .filter(|standing| standing.chat_id == chat_id)
      .map(|standing| standing.id)
      .collect()
  }

  // The game of the room a person is in, from joining until it ends.
  fn game_of(&self, chat_id: i64, person: &str) -> Option<u64> {
    self
      .standings
      .iter()
      .find(|standing| standing.chat_id == chat_id && standing.has(person))
      .map(|standing| standing.id)
  }

  fn people(&mut self, id: u64) -> Vec<(String, String)> {
    self.find(id).map(|standing| standing.people.clone()).unwrap_or_default()
  }

  fn book(&mut self, book_path: &Path) -> Option<Arc<dyn WordBook>> {
    self
      .book
      .get_or_insert_with(|| {
        if !book_path.exists() {
          return None;
        }

        match SqliteWordBook::open(book_path) {
          Ok(book) => Some(Arc::new(book) as Arc<dyn WordBook>),
          Err(error) => {
            eprintln!("word chain: the word book at {} did not open: {error}", book_path.display());
            None
          }
        }
      })
      .clone()
  }

  // Lines one step of a game says. Everything one step says goes out as one message, in the order it was said,
  // opening on the game's name where another game stood beside it as the step began. A game that ends leaves its
  // room at once, and its last lines still go out.
  fn heard(&mut self, id: u64, lines: Vec<Line>, over: bool) {
    let Some(chat_id) = self.find(id).map(|standing| standing.chat_id) else {
      return;
    };
    let beside = self.in_room(chat_id).len() > 1;
    let Some(standing) = self.find(id) else {
      return;
    };
    let first = standing.pending.is_empty();

    if first {
      standing.named = beside;
    }

    standing.pending.extend(lines);

    if first {
      self.batches.push(id);
    }

    if over && let Some(at) = self.standings.iter().position(|standing| standing.id == id) {
      let ended = self.standings.remove(at);

      self.closing.push(ended);
    }
  }

  // What the game of a standing said and the alarms it set, taken in as lines and alarms. A line is made as its
  // event is read, so it reads the game as it stood then.
  fn pump(&mut self, id: u64, book: &dyn WordBook) {
    let Some(standing) = self.find(id) else {
      return;
    };
    let Some(game) = standing.game.as_mut() else {
      return;
    };
    let said = game.take_said();
    let armed = game.take_armed();

    self.alarms.extend(armed.into_iter().map(|alarm| (id, Fire::Game(alarm.id), alarm.ms)));

    for one in said {
      if let GameEvent::Joined { player } = &one.event
        && !player.robot
        && let Some(standing) = self.find(id)
        && !standing.has(&player.id)
      {
        standing.people.push((player.id.clone(), player.name.clone()));
      }

      let over = matches!(one.event, GameEvent::Over { .. });

      self.heard(id, event_lines(&one.event, &one.players, book), over);
    }
  }

  // Runs one call on a standing's game and takes in what it said.
  fn play<T>(
    &mut self,
    id: u64,
    book: &dyn WordBook,
    call: impl FnOnce(&mut WordChainGame) -> T,
  ) -> Option<T> {
    let answer = self.find(id)?.game.as_mut().map(call);

    self.pump(id, book);
    answer
  }

  // The messages the step said, those said straight out first, and the alarms it set.
  fn settle(&mut self, robot_name: &str) -> (Vec<Message>, Vec<Armed>) {
    let mut messages = std::mem::take(&mut self.direct);

    for id in std::mem::take(&mut self.batches) {
      let Some(standing) = self.find(id) else {
        continue;
      };

      if standing.pending.is_empty() {
        continue;
      }

      let name = standing.named.then(|| game_line(&standing.people, robot_name));
      let lines: Vec<Line> = name.into_iter().chain(standing.pending.drain(..)).collect();

      messages.push(Message {
        chat_id: standing.chat_id,
        lines,
      });
    }

    self.closing.clear();

    (messages, std::mem::take(&mut self.alarms))
  }
}

impl WordChainRooms {
  /// The games, their book installed at `book_path`, their messages going out through `say`, on the process's
  /// own clock, the robot playing under `robot_name`.
  pub fn new(book_path: PathBuf, say: Say, robot_name: String) -> Self {
    Self::of(Parts {
      book_path,
      say,
      clock: Arc::new(TaskClock),
      tuning: Tuning::default(),
      robot_name,
    })
  }

  pub fn of(parts: Parts) -> Self {
    Self {
      inner: Arc::new(Inner {
        state: Mutex::new(State::default()),
        book_path: parts.book_path,
        say: parts.say,
        clock: parts.clock,
        tuning: parts.tuning,
        robot_name: parts.robot_name,
      }),
    }
  }

  fn state(&self) -> MutexGuard<'_, State> {
    self.inner.state.lock().unwrap_or_else(PoisonError::into_inner)
  }

  /// Every game played in a room let go without another turn, as the room's registration is removed: nothing
  /// more is said there, and an alarm one set finds it gone.
  pub fn forget(&self, chat_id: i64) {
    let mut state = self.state();

    state.standings.retain(|standing| standing.chat_id != chat_id);
    state.closing.retain(|standing| standing.chat_id != chat_id);
    state.direct.retain(|message| message.chat_id != chat_id);
  }

  // One step on the games: what it said goes out and the alarms it set are set once the games are let go, so
  // neither a send nor a timer ever runs inside it.
  fn step<T>(&self, work: impl FnOnce(&mut State, &Inner) -> T) -> T {
    let (answer, messages, alarms) = {
      let mut state = self.state();
      let answer = work(&mut state, &self.inner);
      let (messages, alarms) = state.settle(&self.inner.robot_name);

      (answer, messages, alarms)
    };

    for message in messages {
      (self.inner.say)(message);
    }

    for (id, fire, ms) in alarms {
      let rooms = Arc::downgrade(&self.inner);

      self.inner.clock.after(ms, Box::new(move || Self::fired(&rooms, id, fire)));
    }

    answer
  }

  fn fired(rooms: &Weak<Inner>, id: u64, fire: Fire) {
    if let Some(inner) = rooms.upgrade() {
      Self { inner }.fire(id, fire);
    }
  }

  /// Whether a game stands in the room.
  pub fn playing(&self, chat_id: i64) -> bool {
    !self.state().in_room(chat_id).is_empty()
  }

  /// Opens a game in the room; what it answers is what the caller reports, the refusal otherwise.
  pub fn start(&self, request: WordChainStart) -> Result<Answer, Answer> {
    self.step(|state, inner| state.start(inner, request))
  }

  /// Ends the game the person is in, or with nobody named the room's only game. Refused where the room holds
  /// none, the person is in none, or several stand and nobody is named.
  pub fn stop(&self, chat_id: i64, player: Option<i64>) -> Result<Answer, Answer> {
    self.step(|state, inner| state.stop(inner, chat_id, player))
  }

  /// A room's message, read against the games standing there. True where it was a game's — a join, a start, a
  /// move, a give-up, a stop, an opening word — so it is taken no further; false where it is talk.
  pub fn take(&self, line: &RoomLine<'_>) -> bool {
    self.step(|state, inner| state.take(inner, line))
  }

  fn fire(&self, id: u64, fire: Fire) {
    self.step(|state, inner| state.fire(inner, id, fire));
  }
}

impl State {
  fn start(&mut self, inner: &Inner, request: WordChainStart) -> Result<Answer, Answer> {
    let Some(book) = self.book(&inner.book_path) else {
      return Err(Answer::of(
        "no-book",
        format!(
          "No word book is installed at {}; install one with bmo-word-chain install.",
          inner.book_path.display()
        ),
      ));
    };
    let chat_id = request.chat_id;
    let starter = request.starter.clone();

    if let Some(starter) = &starter
      && let Some(theirs) = self.game_of(chat_id, &starter.id.to_string())
    {
      let people = self.people(theirs);

      return Err(Answer::of(
        "in-game",
        format!(
          "{} is already in the game {} of this room.",
          starter.name,
          game_words(&people, &inner.robot_name)
        ),
      ));
    }

    let rules = ChainRules {
      loanwords: request.loanwords,
      manner: request.manner,
    };
    let unusable = request.opening.as_deref().and_then(|word| opening_refusal(book.as_ref(), word, rules));

    if let (Some(unusable), None) = (&unusable, &starter) {
      return Err(Answer::of(
        "unusable",
        format!(
          "No game was opened: {}. Open one again with another opening word.",
          unusable_words(unusable)
        ),
      ));
    }

    self.next_id += 1;

    let id = self.next_id;
    let opening = request.opening.clone();

    self.standings.push(Standing {
      id,
      chat_id,
      request,
      game: None,
      people: starter.iter().map(|starter| (starter.id.to_string(), starter.name.clone())).collect(),
      pending: Vec::new(),
      named: false,
      wait: None,
    });

    if let (Some(unusable), Some(starter)) = (unusable, &starter) {
      self.heard(id, vec![ask_again(&unusable, &starter.name)], false);
      self.next_id += 1;

      let wait = self.next_id;

      if let Some(standing) = self.find(id) {
        standing.wait = Some(wait);
      }

      self.alarms.push((id, Fire::Wait(wait), WORD_CHAIN.times.idle_seconds * 1000));

      return Ok(Answer::of(
        "asked-again",
        format!(
          "{}, so {} was asked in the room for another opening word; the game opens on a usable one.",
          unusable_words(&unusable),
          starter.name
        ),
      ));
    }

    self.open(inner, id, &book, opening);

    let count = self.in_room(chat_id).len();
    let more = if count > 1 { format!(" {count} games stand in this room.") } else { String::new() };

    Ok(Answer::of(
      "opened",
      format!("A word-chain game is open in room {chat_id}, taking players.{more}"),
    ))
  }

  // Opens a standing game to joiners on the given opening word: the call for players goes out with the first
  // joiners in it, the robot and whoever asked.
  fn open(&mut self, inner: &Inner, id: u64, book: &Arc<dyn WordBook>, opening: Option<String>) {
    let Some(standing) = self.find(id) else {
      return;
    };
    let request = standing.request.clone();
    let rules = standing.rules();
    let times = &WORD_CHAIN.times;
    // Rounded as a person counts, and held to the range a turn may take.
    let seconds = request.turn_seconds.map(|seconds| {
      let rounded = (seconds + 0.5).floor().clamp(0.0, u64::MAX as f64) as u64;

      rounded.clamp(times.least_turn_seconds, times.most_turn_seconds)
    });
    let game = WordChainGame::new(
      Arc::clone(book),
      GameSettings {
        rules,
        turn_ms: seconds.map(|seconds| seconds * 1000),
        idle_ms: times.idle_seconds * 1000,
        recruit_ms: inner.tuning.recruit_ms,
        robot_ms: inner.tuning.robot_ms,
        round_answers: inner.tuning.round_answers,
        robot: request.robot.map(|difficulty| RobotSeat {
          id: "robot".to_owned(),
          name: inner.robot_name.clone(),
          difficulty,
        }),
        opening: opening.clone().map(|word| Opening {
          word,
          by: request.starter.as_ref().map(|starter| starter.id.to_string()),
        }),
      },
    );
    // Beside another game recruiting, a bare join word cannot say which game it is for.
    let beside = self
      .standings
      .iter()
      .any(|other| other.chat_id == request.chat_id && other.id != id && other.recruiting());
    let people = self.people(id);
    let join_name = beside.then(|| names_of(&people, &inner.robot_name).swap_remove(0));

    if let Some(standing) = self.find(id) {
      standing.game = Some(game);
    }

    let call = Call {
      robot: request.robot,
      robot_name: &inner.robot_name,
      loanwords: request.loanwords,
      manner: request.manner,
      seconds,
      wait_ms: inner.tuning.recruit_ms,
      opening: opening.as_deref(),
      join_name,
    };

    self.heard(id, vec![opening_line(&call)], false);
    self.play(id, book.as_ref(), WordChainGame::open);

    if let Some(starter) = &request.starter {
      self.play(id, book.as_ref(), |game| game.join(&starter.id.to_string(), &starter.name));
    }
  }

  // Ends a game where it stands; one still waiting for its opening word folds.
  fn end(&mut self, inner: &Inner, id: u64) {
    let has_game = self.find(id).is_some_and(|standing| standing.game.is_some());

    if has_game {
      if let Some(book) = self.book(&inner.book_path) {
        self.play(id, book.as_ref(), WordChainGame::stop);
      }

      return;
    }

    if let Some(standing) = self.find(id) {
      standing.wait = None;
    }

    self.heard(id, vec![folded()], true);
  }

  fn stop(&mut self, inner: &Inner, chat_id: i64, player: Option<i64>) -> Result<Answer, Answer> {
    let games = self.in_room(chat_id);
    let target = match player {
      None if games.len() == 1 => Some(games[0]),
      None => None,
      Some(player) => self.game_of(chat_id, &player.to_string()),
    };
    let Some(target) = target else {
      return Err(if games.is_empty() {
        Answer::of("none", "No word-chain game stands in this room.".to_owned())
      } else if player.is_none() {
        let names = games
          .iter()
          .map(|&id| game_words(&self.people(id), &inner.robot_name))
          .collect::<Vec<_>>()
          .join(", ");

        Answer::of(
          "several",
          format!(
            "{} games stand in this room: {names}. Name the person who asked to stop as player.",
            games.len()
          ),
        )
      } else {
        Answer::of("not-theirs", "No word-chain game of this room has that person in it.".to_owned())
      });
    };
    let name = game_words(&self.people(target), &inner.robot_name);

    self.end(inner, target);

    Ok(Answer::of(
      "stopped",
      format!("The word-chain game {name} of room {chat_id} is stopped."),
    ))
  }

  fn take(&mut self, inner: &Inner, line: &RoomLine<'_>) -> bool {
    let Some(sender) = line.sender_id else {
      return false;
    };
    let games = self.in_room(line.chat_id);

    if games.is_empty() {
      return false;
    }

    let said = line.text.trim().trim_end_matches(['.', '!', '?', '~']).trim();
    let person = sender.to_string();
    let mine = self.game_of(line.chat_id, &person);
    let words = &WORD_CHAIN.words;
    let stopping = addressed(said, &words.stop);

    // A player's stop is for the game they are in; the owner, in none, names the one meant.
    if let (Some(""), Some(mine)) = (stopping.as_deref(), mine) {
      self.end(inner, mine);

      return true;
    }

    if let (Some(name), None, true) = (&stopping, mine, line.owner) {
      return self.pick(inner, line.chat_id, &games, name, &Act::End);
    }

    let Some(mine) = mine else {
      let joining = addressed(said, &words.join);
      let recruiting: Vec<u64> = self
        .standings
        .iter()
        .filter(|standing| standing.chat_id == line.chat_id && standing.recruiting())
        .map(|standing| standing.id)
        .collect();
      let act = Act::Join(person, line.sender_name.to_owned());

      return match joining {
        Some(name) if !recruiting.is_empty() => self.pick(inner, line.chat_id, &recruiting, &name, &act),
        _ => false,
      };
    };
    let Some(book) = self.book(&inner.book_path) else {
      return false;
    };
    let phase = self.find(mine).and_then(|standing| standing.game.as_ref().map(WordChainGame::phase));

    match phase {
      None => self.offered(inner, mine, said, &book),
      Some(Phase::Recruiting) if holds(&words.join, said) => true,
      Some(Phase::Recruiting) if holds(&words.begin, said) => {
        self.play(mine, book.as_ref(), WordChainGame::begin);
        true
      }
      Some(Phase::Recruiting) => false,
      Some(_) if holds(&words.give_up, said) => {
        self.play(mine, book.as_ref(), |game| game.give_up(&person)).unwrap_or(false)
      }
      Some(_) => self.play(mine, book.as_ref(), |game| game.submit(&person, said)).unwrap_or(false),
    }
  }

  // The game a join or the owner's stop is for: the only one, or the one whose player the message names. A bare
  // word among several, or a name several fit, is answered with the games to choose from; a name none fits leaves
  // the message as talk.
  fn pick(&mut self, inner: &Inner, chat_id: i64, games: &[u64], name: &str, act: &Act) -> bool {
    let fitting: Vec<u64> = games
      .iter()
      .copied()
      .filter(|&id| {
        name.is_empty() || names_of(&self.people(id), &inner.robot_name).iter().any(|one| fits(name, one))
      })
      .collect();

    match fitting.as_slice() {
      [] => false,
      [only] => {
        match act {
          Act::End => self.end(inner, *only),
          Act::Join(person, sender) => {
            if let Some(book) = self.book(&inner.book_path) {
              self.play(*only, book.as_ref(), |game| game.join(person, sender));
            }
          }
        }

        true
      }
      [first, ..] => {
        let example = names_of(&self.people(*first), &inner.robot_name).swap_remove(0);
        let named: Vec<Vec<String>> =
          fitting.iter().map(|&id| names_of(&self.people(id), &inner.robot_name)).collect();
        let doing = match act {
          Act::End => "stop",
          Act::Join(..) => "join",
        };

        self.direct.push(Message {
          chat_id,
          lines: vec![pick(doing, &named, &example)],
        });

        true
      }
    }
  }

  // A word the person who asked offers to open their game on, while it waits for one. Only one Hangul word of two
  // syllables or more is an offer, said alone or after a begin or offer word; a bare begin word is answered with
  // what is wanted, and anything else they say is talk.
  fn offered(&mut self, inner: &Inner, id: u64, said: &str, book: &Arc<dyn WordBook>) -> bool {
    let words = &WORD_CHAIN.words;
    let offers: Vec<String> = words.begin.iter().chain(&words.offer).cloned().collect();
    let after = addressed(said, &offers);
    let Some(standing) = self.find(id) else {
      return false;
    };
    let asker = standing
      .request
      .starter
      .as_ref()
      .map(|starter| starter.name.clone())
      .unwrap_or_default();
    let rules = standing.rules();

    if after.as_deref() == Some("") {
      self.heard(id, vec![ask_first(&asker)], false);

      return true;
    }

    let word = after.unwrap_or_else(|| said.to_owned());

    if !is_hangul_word(&word) || word.chars().count() < 2 {
      return false;
    }

    if let Some(unusable) = opening_refusal(book.as_ref(), &word, rules) {
      self.heard(id, vec![ask_again_bare(&unusable)], false);

      return true;
    }

    if let Some(standing) = self.find(id) {
      standing.wait = None;
    }

    self.open(inner, id, book, Some(word));

    true
  }

  fn fire(&mut self, inner: &Inner, id: u64, fire: Fire) {
    if !self.standings.iter().any(|standing| standing.id == id) {
      return;
    }

    match fire {
      Fire::Game(alarm) => {
        if let Some(book) = self.book(&inner.book_path) {
          self.play(id, book.as_ref(), |game| game.fire(alarm));
        }
      }
      Fire::Wait(wait) => {
        let waiting = self.find(id).is_some_and(|standing| standing.wait == Some(wait));

        if waiting {
          if let Some(standing) = self.find(id) {
            standing.wait = None;
          }

          self.heard(id, vec![no_opening()], true);
        }
      }
    }
  }
}
