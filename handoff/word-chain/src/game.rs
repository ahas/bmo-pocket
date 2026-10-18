//! One game of the word chain among the people of a room and, where asked, a robot playing from the word book.
//! The game judges every move by the book alone, so a move costs a lookup and nothing else. It says what
//! happened as events, each with its players as they stood then, and asks for time as alarms its host
//! sets and hands back when they come due; how an event reads in a room, and the clock, are its host's.

use std::collections::HashSet;
use std::sync::Arc;

use rand::seq::SliceRandom;

use crate::book::{ChainRules, WordBook, openings_after};
use crate::hangul::is_hangul_word;
use crate::policy::WORD_CHAIN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
  Easy,
  Normal,
  Hard,
}

/// The robot's seat in a game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RobotSeat {
  pub id: String,
  pub name: String,
  pub difficulty: Difficulty,
}

/// The word the first round opens on, and the player who gave it, where one was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opening {
  pub word: String,
  pub by: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameSettings {
  pub rules: ChainRules,
  /// How long a player has for one move, or nothing where a move has no limit.
  pub turn_ms: Option<u64>,
  /// How long a move with no limit is waited for before the game closes for want of play.
  pub idle_ms: u64,
  /// How long the game waits for people to join before it starts on its own.
  pub recruit_ms: u64,
  /// How long the robot takes over its move, so a room reads its word after the last one.
  pub robot_ms: u64,
  /// How many words a round's opening word must leave to answer with.
  pub round_answers: u64,
  pub robot: Option<RobotSeat>,
  /// The word the first round opens on, where one was given; the book draws one otherwise. The player
  /// after the one who gave it moves first.
  pub opening: Option<Opening>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elimination {
  Timeout,
  Hanbang,
  GaveUp,
  Stuck,
}

/// Why a word was not taken. The player keeps their turn and may try again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
  OffChain,
  NotInBook,
  Loanword,
  Used,
  Manner,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
  pub id: String,
  pub name: String,
  pub robot: bool,
  pub score: i64,
  pub words: u32,
  /// Why the player is out, or nothing while they play.
  pub out: Option<Elimination>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
  Recruiting,
  Playing,
  Over,
}

/// Why a game ended: one player left, stopped, nobody joined, or nobody moved for too long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
  Decided,
  Stopped,
  Nobody,
  Idle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameEvent {
  Joined {
    player: Player,
  },
  Round {
    word: String,
    openings: Vec<String>,
    first: bool,
  },
  Turn {
    player: Player,
    openings: Vec<String>,
    seconds: Option<u64>,
  },
  Accepted {
    player: Player,
    word: String,
    points: i64,
    openings: Vec<String>,
  },
  Rejected {
    player: Player,
    word: String,
    reason: Rejection,
    openings: Vec<String>,
  },
  Eliminated {
    player: Player,
    reason: Elimination,
    openings: Vec<String>,
    by: Option<Player>,
  },
  Over {
    ranking: Vec<Player>,
    reason: Ending,
  },
}

/// One thing the game said, with its players as they stood when it said it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Said {
  pub event: GameEvent,
  pub players: Vec<Player>,
}

/// Time the game asks for: `fire` it with this id once `ms` have passed. An alarm the game has
/// cancelled since is passed over when fired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alarm {
  pub id: u64,
  pub ms: u64,
}

// What an alarm does when it comes due.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Due {
  Begin,
  Idle,
  Timeout(String),
  Robot(String),
}

pub struct WordChainGame {
  book: Arc<dyn WordBook>,
  settings: GameSettings,
  players: Vec<Player>,
  used: HashSet<String>,
  phase: Phase,
  turn: usize,
  openings: Vec<String>,
  // The one clock a turn or the recruiting runs on, and the robot's pause, each with its alarm's id.
  timer: Option<(u64, Due)>,
  robot_timer: Option<(u64, Due)>,
  next_alarm: u64,
  // The players who went out, first out first, by id.
  fallen: Vec<String>,
  // Whether the game opened with more than one player, which is what decides when it is over.
  contest: bool,
  said: Vec<Said>,
  armed: Vec<Alarm>,
}

impl WordChainGame {
  pub fn new(book: Arc<dyn WordBook>, settings: GameSettings) -> Self {
    Self {
      book,
      settings,
      players: Vec::new(),
      used: HashSet::new(),
      phase: Phase::Recruiting,
      turn: 0,
      openings: Vec::new(),
      timer: None,
      robot_timer: None,
      next_alarm: 0,
      fallen: Vec::new(),
      contest: false,
      said: Vec::new(),
      armed: Vec::new(),
    }
  }

  pub fn phase(&self) -> Phase {
    self.phase
  }

  pub fn players(&self) -> &[Player] {
    &self.players
  }

  /// The player whose move it is, or nothing outside play.
  pub fn current(&self) -> Option<&Player> {
    (self.phase == Phase::Playing).then(|| self.players.get(self.turn)).flatten()
  }

  /// The syllables the next word may start with.
  pub fn openings(&self) -> &[String] {
    &self.openings
  }

  /// What the game has said since this was last asked, in order.
  pub fn take_said(&mut self) -> Vec<Said> {
    std::mem::take(&mut self.said)
  }

  /// The alarms the game has armed since this was last asked.
  pub fn take_armed(&mut self) -> Vec<Alarm> {
    std::mem::take(&mut self.armed)
  }

  fn emit(&mut self, event: GameEvent) {
    self.said.push(Said {
      event,
      players: self.players.clone(),
    });
  }

  fn arm(&mut self, ms: u64, due: Due) -> (u64, Due) {
    self.next_alarm += 1;
    self.armed.push(Alarm {
      id: self.next_alarm,
      ms,
    });

    (self.next_alarm, due)
  }

  /// An alarm the game armed come due.
  pub fn fire(&mut self, alarm: u64) {
    let due = if self.timer.as_ref().is_some_and(|(id, _)| *id == alarm) {
      self.timer.take()
    } else if self.robot_timer.as_ref().is_some_and(|(id, _)| *id == alarm) {
      self.robot_timer.take()
    } else {
      None
    };

    match due.map(|(_, due)| due) {
      Some(Due::Begin) => self.begin(),
      Some(Due::Idle) => self.finish(Ending::Idle),
      Some(Due::Timeout(player)) => self.eliminate(&player, Elimination::Timeout, None),
      Some(Due::Robot(player)) => self.robot_move(&player),
      None => {}
    }
  }

  /// Opens the game to joiners, the robot among them first where it plays, and starts the wait.
  pub fn open(&mut self) {
    if let Some(robot) = self.settings.robot.clone() {
      self.seat(robot.id, robot.name, true);
    }

    self.timer = Some(self.arm(self.settings.recruit_ms, Due::Begin));
  }

  /// Adds a person while the game recruits; false where it no longer does or they are in already.
  pub fn join(&mut self, id: &str, name: &str) -> bool {
    self.seat(id.to_owned(), name.to_owned(), false)
  }

  fn seat(&mut self, id: String, name: String, robot: bool) -> bool {
    if self.phase != Phase::Recruiting || self.players.iter().any(|player| player.id == id) {
      return false;
    }

    let player = Player {
      id,
      name,
      robot,
      score: 0,
      words: 0,
      out: None,
    };

    self.players.push(player.clone());
    self.emit(GameEvent::Joined { player });

    true
  }

  /// Starts play with whoever joined, the robot moving last and the order starting after whoever gave
  /// the opening word; with no person in, the game is over.
  pub fn begin(&mut self) {
    if self.phase != Phase::Recruiting {
      return;
    }

    self.clear_timers();

    if !self.players.iter().any(|player| !player.robot) {
      self.finish(Ending::Nobody);

      return;
    }

    self.players.sort_by_key(|player| player.robot);

    // The one who gave the opening word has had their say; the order starts after them.
    let giver = self.settings.opening.as_ref().and_then(|opening| opening.by.as_ref());

    if let Some(at) = giver.and_then(|by| self.players.iter().position(|player| &player.id == by)) {
      self.players.rotate_left(at + 1);
    }

    self.contest = self.players.len() > 1;
    self.phase = Phase::Playing;
    self.turn = 0;
    self.new_round(true);
  }

  /// A player's word. Only the player whose move it is is heard, and only with a word of two Hangul
  /// syllables or more, which is judged and taken or refused; anything else is not a move and answers
  /// false, so the host passes it on as talk.
  pub fn submit(&mut self, player_id: &str, text: &str) -> bool {
    let word = text.trim();
    let heard = self.current().is_some_and(|player| player.id == player_id && !player.robot)
      && is_hangul_word(word)
      && word.chars().count() >= 2;

    if heard {
      self.judge(self.turn, word);
    }

    heard
  }

  /// A player gives up. The one whose move it was hands the move on with a new round.
  pub fn give_up(&mut self, player_id: &str) -> bool {
    let playing = self.players.iter().any(|one| one.id == player_id && one.out.is_none() && !one.robot);

    if !playing || self.phase != Phase::Playing {
      return false;
    }

    self.eliminate(player_id, Elimination::GaveUp, None);

    true
  }

  /// Ends the game where it stands.
  pub fn stop(&mut self) {
    if self.phase != Phase::Over {
      self.finish(Ending::Stopped);
    }
  }

  fn judge(&mut self, seat: usize, word: &str) {
    if let Some(reason) = self.rejection(word) {
      let player = self.players[seat].clone();
      let openings = self.openings.clone();

      self.emit(GameEvent::Rejected {
        player,
        word: word.to_owned(),
        reason,
        openings,
      });

      return;
    }

    self.clear_timers();
    self.used.insert(word.to_owned());

    let points = i64::try_from(word.chars().count()).unwrap_or(0) * WORD_CHAIN.points.per_syllable;
    let next = openings_after(word);

    self.players[seat].score += points;
    self.players[seat].words += 1;
    self.openings = next.clone();

    let player = self.players[seat].clone();

    self.emit(GameEvent::Accepted {
      player,
      word: word.to_owned(),
      points,
      openings: next.clone(),
    });

    let following = self.next_active(self.turn);
    let rules = self.settings.rules;

    match following {
      // Nothing the book allows answers the word: the next player is out on the spot, with no wait for
      // a clock that could only run down.
      Some(index) if index != self.turn && !self.book.has_follower(&next, rules, &self.used) => {
        self.players[seat].score += WORD_CHAIN.points.hanbang;
        self.turn = index;

        let (out, by) = (self.players[index].id.clone(), self.players[seat].clone());

        self.eliminate(&out, Elimination::Hanbang, Some(by));

        return;
      }
      Some(index) if index != self.turn => self.turn = index,
      // A player alone who has left nothing to follow scores the hanbang and goes on from a new word.
      _ => {
        if !self.book.has_follower(&next, rules, &self.used) {
          self.players[seat].score += WORD_CHAIN.points.hanbang;
          self.new_round(false);

          return;
        }
      }
    }

    self.start_turn();
  }

  fn rejection(&self, word: &str) -> Option<Rejection> {
    let first = word.chars().next().map(String::from).unwrap_or_default();

    if !self.openings.contains(&first) {
      return Some(Rejection::OffChain);
    }

    if self.used.contains(word) {
      return Some(Rejection::Used);
    }

    let Some(entry) = self.book.find(word) else {
      return Some(Rejection::NotInBook);
    };
    let rules = self.settings.rules;

    if entry.loanword && !rules.loanwords {
      return Some(Rejection::Loanword);
    }

    (rules.manner && !self.leaves_follower(word)).then_some(Rejection::Manner)
  }

  // Whether a word, once played, leaves anything to answer it with.
  fn leaves_follower(&self, word: &str) -> bool {
    let mut used = self.used.clone();

    used.insert(word.to_owned());

    self.book.has_follower(&openings_after(word), self.settings.rules, &used)
  }

  fn start_turn(&mut self) {
    let Some(player) = self.current().cloned() else {
      return;
    };
    let turn_ms = self.settings.turn_ms;
    let openings = self.openings.clone();

    self.emit(GameEvent::Turn {
      player: player.clone(),
      openings,
      seconds: turn_ms.map(|ms| (ms + 500) / 1000),
    });

    if player.robot {
      self.robot_timer = Some(self.arm(self.settings.robot_ms, Due::Robot(player.id)));
    } else {
      // No limit puts nobody out, but a game left standing with nobody playing closes.
      self.timer = Some(match turn_ms {
        None => self.arm(self.settings.idle_ms, Due::Idle),
        Some(ms) => self.arm(ms, Due::Timeout(player.id)),
      });
    }
  }

  fn robot_move(&mut self, robot: &str) {
    let Some(seat) = self.players.iter().position(|player| player.id == robot) else {
      return;
    };

    match self.robot_word() {
      Some(word) => self.judge(seat, &word),
      None => self.eliminate(robot, Elimination::Stuck, None),
    }
  }

  // The robot's word: any the rules allow at normal, the one leaving the most answers at easy, shortest
  // first, and the one leaving the fewest at hard.
  fn robot_word(&self) -> Option<String> {
    let rules = self.settings.rules;
    let difficulty = self.settings.robot.as_ref().map_or(Difficulty::Normal, |robot| robot.difficulty);
    let candidates = self.book.sample(&self.openings, rules, &self.used, WORD_CHAIN.rounds.robot_reach);
    let mut playable: Vec<String> = if rules.manner {
      candidates.into_iter().filter(|word| self.leaves_follower(word)).collect()
    } else {
      candidates
    };

    if difficulty == Difficulty::Normal {
      return playable.into_iter().next();
    }

    let answers = |word: &String| self.book.followers(&openings_after(word), rules);

    match difficulty {
      Difficulty::Hard => playable.sort_by_cached_key(answers),
      _ => playable.sort_by_cached_key(|word| (std::cmp::Reverse(answers(word)), word.chars().count())),
    }

    playable.into_iter().next()
  }

  fn eliminate(&mut self, id: &str, reason: Elimination, by: Option<Player>) {
    let Some(seat) = self.players.iter().position(|player| player.id == id) else {
      return;
    };

    if self.players[seat].out.is_some() || self.phase != Phase::Playing {
      return;
    }

    let was_current = self.current().is_some_and(|player| player.id == id);

    self.clear_timers();
    self.players[seat].out = Some(reason);
    self.fallen.push(id.to_owned());

    let (player, openings) = (self.players[seat].clone(), self.openings.clone());

    self.emit(GameEvent::Eliminated {
      player,
      reason,
      openings,
      by,
    });

    let standing: Vec<&Player> = self.players.iter().filter(|one| one.out.is_none()).collect();

    if standing.is_empty()
      || (self.contest && standing.len() == 1)
      || !standing.iter().any(|one| !one.robot)
    {
      self.finish(Ending::Decided);

      return;
    }

    if was_current {
      self.turn = self.next_active(self.turn).unwrap_or(self.turn);
      self.new_round(false);
    } else {
      self.start_turn();
    }
  }

  // A fresh word from the book to start from, one that can be answered.
  fn new_round(&mut self, first: bool) {
    let rules = self.settings.rules;
    let answerable = |word: &String| {
      word.chars().count() <= 3
        && self.book.followers(&openings_after(word), rules) >= self.settings.round_answers
    };
    let mut pick = if first {
      self.settings.opening.as_ref().map(|opening| opening.word.clone())
    } else {
      None
    };

    if pick.is_none() {
      let mut syllables = WORD_CHAIN.rounds.syllables.clone();

      syllables.shuffle(&mut rand::rng());

      for syllable in syllables {
        let drawn = self.book.sample(&[syllable], rules, &self.used, WORD_CHAIN.rounds.robot_reach);

        pick = drawn.into_iter().find(&answerable);

        if pick.is_some() {
          break;
        }
      }
    }

    let Some(pick) = pick else {
      self.finish(Ending::Decided);

      return;
    };

    self.used.insert(pick.clone());
    self.openings = openings_after(&pick);

    let openings = self.openings.clone();

    self.emit(GameEvent::Round {
      word: pick,
      openings,
      first,
    });
    self.start_turn();
  }

  // The next seat after the given one still in play, which may be the seat itself.
  fn next_active(&self, from: usize) -> Option<usize> {
    let count = self.players.len();

    (1..=count)
      .map(|step| (from + step) % count)
      .find(|&index| self.players[index].out.is_none())
  }

  fn finish(&mut self, reason: Ending) {
    self.clear_timers();

    let standing: Vec<usize> =
      (0..self.players.len()).filter(|&at| self.players[at].out.is_none()).collect();

    if reason == Ending::Decided && self.contest && standing.len() == 1 {
      self.players[standing[0]].score += WORD_CHAIN.points.survivor;
    }

    self.phase = Phase::Over;

    // Those still in first, best score first; then those who went out, the last out first.
    let mut ranking: Vec<Player> = standing.iter().map(|&at| self.players[at].clone()).collect();

    ranking.sort_by_key(|player| std::cmp::Reverse(player.score));
    ranking.extend(
      self
        .fallen
        .iter()
        .rev()
        .filter_map(|id| self.players.iter().find(|player| &player.id == id).cloned()),
    );

    self.emit(GameEvent::Over { ranking, reason });
  }

  fn clear_timers(&mut self) {
    self.timer = None;
    self.robot_timer = None;
  }
}
