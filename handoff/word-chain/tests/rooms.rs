//! The games as a room plays them: what each step says, in one message a step, each line by its code and the
//! facts it reads, on a clock that fires only what the test asks for. Its words are the fixture's
//! (`tests/fixtures/rooms.toml`).

use std::sync::{Arc, LazyLock, Mutex};

use serde::Deserialize;
use serde_json::{Value, json};
use word_chain::lines::{Line, Message};
use word_chain::rooms::{Clock, Parts, RoomLine, Starter, Tuning, WordChainRooms, WordChainStart};
use word_chain::{Difficulty, SqliteWordBook};

#[derive(Deserialize)]
struct Fixture {
  words: Words,
  meanings: Meanings,
}

#[derive(Deserialize)]
struct Words {
  train: String,
  ticket: String,
  leopard: String,
  culprit: String,
  train_hands: String,
  ticket_hands: String,
  leopard_hands: String,
  join: String,
  begin: String,
  begin_loud: String,
  give_up: String,
  stop: String,
  stop_dave: String,
  join_erin: String,
  join_dave: String,
  hello: String,
  laughter: String,
  not_in_book: String,
  wait_a_moment: String,
  me_too: String,
  begin_leopard: String,
}

#[derive(Deserialize)]
struct Meanings {
  ticket: String,
  train: String,
}

static FIXTURE: LazyLock<Fixture> =
  LazyLock::new(|| toml::from_str(include_str!("fixtures/rooms.toml")).expect("the rooms fixture reads"));

fn w() -> &'static Words {
  &FIXTURE.words
}

// The name the robot plays under.
const ROBOT: &str = "Robot";

type Timers = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;

// A clock whose alarms wait until the test fires them all.
struct HandClock(Timers);

impl Clock for HandClock {
  fn after(&self, _ms: u64, fire: Box<dyn FnOnce() + Send>) {
    self.0.lock().expect("the timers").push(fire);
  }
}

struct Room {
  games: WordChainRooms,
  sent: Arc<Mutex<Vec<Message>>>,
  timers: Timers,
  _book: tempfile::TempDir,
}

impl Room {
  fn new() -> Self {
    let book = tempfile::tempdir().expect("a scratch directory");
    let path = book.path().join("ko.sqlite");
    let words = w();

    SqliteWordBook::write_file(
      &path,
      &[
        (&words.train, false),
        (&words.ticket, false),
        (&words.leopard, false),
        (&words.culprit, false),
      ],
      &[
        (&words.ticket, &FIXTURE.meanings.ticket, false),
        (&words.train, &FIXTURE.meanings.train, true),
      ],
    )
    .expect("the fixture book");

    Self::on(path, book)
  }

  fn on(book_path: std::path::PathBuf, book: tempfile::TempDir) -> Self {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let timers: Timers = Arc::default();
    let said = Arc::clone(&sent);
    let games = WordChainRooms::of(Parts {
      book_path,
      say: Arc::new(move |message| said.lock().expect("the lines").push(message)),
      clock: Arc::new(HandClock(Arc::clone(&timers))),
      tuning: Tuning {
        round_answers: 1,
        ..Tuning::default()
      },
      robot_name: ROBOT.to_owned(),
    });

    Self {
      games,
      sent,
      timers,
      _book: book,
    }
  }

  // Each message sent, as its lines.
  fn sent(&self) -> Vec<Vec<Line>> {
    self
      .sent
      .lock()
      .expect("the lines")
      .iter()
      .map(|message| message.lines.clone())
      .collect()
  }

  fn last(&self) -> Vec<Line> {
    self.sent().last().cloned().unwrap_or_default()
  }

  fn fire_all(&self) {
    let due: Vec<_> = self.timers.lock().expect("the timers").drain(..).collect();

    for fire in due {
      fire();
    }
  }

  fn take(&self, text: &str, sender: i64, name: &str) -> bool {
    self.take_in(7, text, sender, name)
  }

  fn take_in(&self, chat_id: i64, text: &str, sender: i64, name: &str) -> bool {
    self.games.take(&RoomLine {
      chat_id,
      sender_id: Some(sender),
      sender_name: name,
      text,
      owner: sender == 50,
    })
  }
}

fn start(chat_id: i64, starter: Option<(i64, &str)>, robot: Option<Difficulty>) -> WordChainStart {
  WordChainStart {
    chat_id,
    starter: starter.map(|(id, name)| Starter {
      id,
      name: name.to_owned(),
    }),
    robot,
    turn_seconds: Some(30.0),
    loanwords: true,
    manner: false,
    opening: None,
  }
}

fn untimed(chat_id: i64, starter: Option<(i64, &str)>, opening: Option<&str>) -> WordChainStart {
  WordChainStart {
    turn_seconds: None,
    opening: opening.map(str::to_owned),
    ..start(chat_id, starter, None)
  }
}

fn line(code: &'static str, fields: Value) -> Line {
  Line::of(code, fields)
}

fn game(names: &[&str]) -> Line {
  line("game", json!({ "names": names, "listed": 3 }))
}

fn joined(name: &str, count: usize) -> Line {
  line("joined", json!({ "name": name, "count": count }))
}

fn started(order: &[&str]) -> Line {
  line("started", json!({ "order": order }))
}

fn first_word(word: &str) -> Line {
  line("first-word", json!({ "word": word }))
}

fn meaning(word: &str, meaning: &str) -> Line {
  line("meaning", json!({ "word": word, "meaning": meaning }))
}

fn turn(name: &str, opening: &str, seconds: Option<u64>) -> Line {
  line("turn", json!({ "name": name, "openings": [opening], "seconds": seconds }))
}

fn taken(word: &str, points: i64) -> Line {
  line("taken", json!({ "word": word, "points": points }))
}

fn ranking(place: usize, name: &str, score: i64, words: usize) -> Line {
  line("ranking", json!({ "place": place, "name": name, "score": score, "words": words }))
}

fn ticket_meaning() -> Line {
  meaning(&w().ticket, &FIXTURE.meanings.ticket)
}

#[test]
fn a_rooms_game_takes_joins_moves_and_give_ups_and_leaves_talk_alone_in_one_message_a_step() {
  let room = Room::new();
  let w = w();

  assert!(room.games.start(start(7, Some((1, "Erin")), None)).is_ok());
  assert_eq!(room.games.start(start(7, Some((1, "Erin")), None)).unwrap_err().code, "in-game");
  assert_eq!(room.sent()[0][0].code, "call");
  assert_eq!(room.sent()[0].last(), Some(&joined("Erin", 1)));
  assert!(room.take(&w.join, 2, "Frank"));
  assert_eq!(room.last(), [joined("Frank", 2)]);
  assert!(!room.take(&w.hello, 2, "Frank"));
  assert!(!room.take(&w.begin, 3, "Onlooker"));
  assert!(room.take(&w.begin_loud, 1, "Erin"));
  assert_eq!(
    room.last(),
    [
      started(&["Erin", "Frank"]),
      first_word(&w.train),
      turn("Erin", &w.train_hands, Some(30))
    ]
  );

  assert!(room.take(&w.ticket, 1, "Erin"));
  assert!(!room.take(&w.laughter, 2, "Frank"));
  assert!(room.take(&w.train, 2, "Frank"));
  assert!(room.take(&w.not_in_book, 2, "Frank"));
  // A word that is not basic comes with its meaning; the train above, a basic word, came without. A word off the
  // chain is refused with where the chain stands.
  let sent = room.sent();

  assert_eq!(
    sent[sent.len() - 3..],
    [
      vec![taken(&w.ticket, 20), ticket_meaning(), turn("Frank", &w.ticket_hands, Some(30))],
      vec![line("off-chain", json!({ "word": w.train, "openings": [w.ticket_hands] }))],
      vec![line("not-in-book", json!({ "word": w.not_in_book }))],
    ]
  );

  assert!(room.take(&w.give_up, 2, "Frank"));
  assert_eq!(
    room.last(),
    [
      line("gave-up", json!({ "name": "Frank" })),
      line("decided", json!({})),
      ranking(1, "Erin", 70, 1),
      ranking(2, "Frank", 0, 0)
    ]
  );
  assert!(!room.games.playing(7));
  room.fire_all();
}

#[test]
fn a_game_with_nobody_but_the_robot_folds_and_one_is_stopped_by_its_words() {
  let room = Room::new();

  assert!(room.games.start(start(7, None, Some(Difficulty::Normal))).is_ok());
  room.fire_all();
  assert_eq!(room.last(), [line("nobody", json!({}))]);
  assert_eq!(room.sent()[room.sent().len() - 2].last(), Some(&joined(ROBOT, 1)));

  assert!(room.games.start(start(7, Some((1, "Erin")), None)).is_ok());
  assert!(!room.take(&w().stop, 9, "Stranger"));
  assert!(room.take(&w().stop, 1, "Erin"));
  assert!(!room.games.playing(7));
}

#[test]
fn a_game_with_no_limit_says_so_names_no_seconds_and_closes_for_want_of_play() {
  let room = Room::new();

  assert!(room.games.start(untimed(7, Some((1, "Erin")), None)).is_ok());

  let call = &room.sent()[0][0];

  assert_eq!(
    (call.code, &call.fields["seconds"], &call.fields["idle_minutes"]),
    ("call", &json!(null), &json!(10))
  );
  room.take(&w().begin, 1, "Erin");
  assert_eq!(room.last().last(), Some(&turn("Erin", &w().train_hands, None)));
  room.fire_all();
  assert_eq!(room.last(), [line("idle", json!({ "minutes": 10 })), ranking(1, "Erin", 0, 0)]);
  assert!(!room.games.playing(7));
}

#[test]
fn a_game_opens_on_the_word_its_asker_gave_with_the_next_player_first() {
  let room = Room::new();
  let w = w();

  assert!(room.games.start(untimed(7, Some((1, "Erin")), Some(&w.ticket))).is_ok());
  room.take(&w.join, 2, "Frank");
  room.take(&w.begin, 1, "Erin");
  assert_eq!(room.sent()[0][0].fields["opening"], json!(w.ticket));
  assert_eq!(
    room.last(),
    [
      started(&["Frank", "Erin"]),
      first_word(&w.ticket),
      ticket_meaning(),
      turn("Frank", &w.ticket_hands, None)
    ]
  );
  assert!(room.games.stop(7, None).is_ok());
}

#[test]
fn an_opening_word_nothing_follows_is_not_replaced_and_the_asker_is_asked_for_another() {
  let room = Room::new();
  let w = w();

  assert_eq!(room.games.start(untimed(8, None, Some(&w.culprit))).unwrap_err().code, "unusable");
  assert!(room.games.start(untimed(8, Some((1, "Erin")), Some(&w.culprit))).is_ok());

  let unusable = json!({ "reason": "no-follower", "word": w.culprit });

  assert_eq!(room.sent(), [vec![line("ask-again", json!({ "unusable": unusable, "name": "Erin" }))]]);
  assert!(!room.take_in(8, &w.wait_a_moment, 1, "Erin"));
  assert!(!room.take_in(8, &w.leopard, 2, "Frank"));
  assert!(room.take_in(8, &w.culprit, 1, "Erin"));
  assert_eq!(room.last(), [line("ask-again-bare", json!({ "unusable": unusable }))]);
  // A bare begin word is told a word is wanted; the word may come after it.
  assert!(room.take_in(8, &w.begin, 1, "Erin"));
  assert_eq!(room.last(), [line("ask-first", json!({ "name": "Erin" }))]);
  assert!(room.take_in(8, &w.begin_leopard, 1, "Erin"));

  let opened = room.last();

  assert_eq!((opened[0].code, &opened[0].fields["opening"]), ("call", &json!(w.leopard)));
  assert_eq!(opened.last(), Some(&joined("Erin", 1)));
  assert!(room.games.stop(8, Some(1)).is_ok());

  assert!(room.games.start(untimed(9, Some((1, "Erin")), Some(&w.culprit))).is_ok());
  room.fire_all();
  // The wait ends as a step of its own, after the request for a word went out.
  assert_eq!(room.last(), [line("no-opening", json!({}))]);
  assert!(!room.games.playing(9));
}

#[test]
fn two_games_stand_in_one_room_each_hearing_its_own_players_named_while_both_stand_and_stopped_apart() {
  let room = Room::new();
  let w = w();
  let open = |id: i64, name: &str| {
    room.games.start(WordChainStart {
      turn_seconds: None,
      ..start(7, Some((id, name)), Some(Difficulty::Normal))
    })
  };

  assert!(open(1, "Erin").is_ok());
  assert_eq!((room.last()[0].code, &room.last()[0].fields["join_name"]), ("call", &json!(null)));
  assert!(open(3, "Dave").is_ok());
  assert!(open(1, "Erin").is_err());

  // The second call names its game, and the robot is in both.
  let second = room.last();

  assert_eq!(second[0], game(&["Dave"]));
  assert_eq!((second[1].code, &second[1].fields["join_name"]), ("call", &json!("Dave")));
  assert!(second.windows(2).any(|pair| pair == [joined(ROBOT, 1), joined("Dave", 2)]));

  assert!(room.take(&w.join, 2, "Frank"));
  assert_eq!(
    room.last(),
    [line("pick", json!({ "for": "join", "games": [["Erin"], ["Dave"]], "example": "Erin" }))]
  );
  assert!(!room.take(&w.me_too, 2, "Frank"));
  assert!(room.take(&w.join_erin, 2, "Frank"));
  assert!(!room.take(&w.join_dave, 2, "Frank"));
  assert_eq!(room.last(), [game(&["Erin", "Frank"]), joined("Frank", 3)]);

  room.take(&w.begin, 1, "Erin");
  room.take(&w.begin, 3, "Dave");

  let sent = room.sent();

  assert_eq!(
    sent[sent.len() - 2..],
    [
      vec![
        game(&["Erin", "Frank"]),
        started(&["Erin", "Frank", ROBOT]),
        first_word(&w.train),
        turn("Erin", &w.train_hands, None)
      ],
      vec![
        game(&["Dave"]),
        started(&["Dave", ROBOT]),
        first_word(&w.train),
        turn("Dave", &w.train_hands, None)
      ],
    ]
  );

  // Each game keeps its own words played: the ticket is taken in both.
  assert!(room.take(&w.ticket, 3, "Dave"));
  assert!(room.take(&w.ticket, 1, "Erin"));

  let sent = room.sent();

  assert_eq!(
    sent[sent.len() - 2..],
    [
      vec![
        game(&["Dave"]),
        taken(&w.ticket, 20),
        ticket_meaning(),
        turn(ROBOT, &w.ticket_hands, None)
      ],
      vec![
        game(&["Erin", "Frank"]),
        taken(&w.ticket, 20),
        ticket_meaning(),
        turn("Frank", &w.ticket_hands, None)
      ],
    ]
  );

  assert_eq!(room.games.stop(7, None).unwrap_err().code, "several");
  assert_eq!(room.games.stop(7, Some(9)).unwrap_err().code, "not-theirs");
  assert!(room.take(&w.stop, 50, "Owner"));
  assert_eq!(
    room.last(),
    [line(
      "pick",
      json!({ "for": "stop", "games": [["Erin", "Frank"], ["Dave"]], "example": "Erin" })
    )]
  );

  assert!(room.take(&w.stop, 2, "Frank"));
  assert_eq!(
    room.last()[..3],
    [game(&["Erin", "Frank"]), line("stopped", json!({})), ranking(1, "Erin", 20, 1)]
  );

  // The game left alone goes on, and is no longer named.
  room.fire_all();
  assert_eq!(
    room.last(),
    [
      line("taken-robot", json!({ "name": ROBOT, "word": w.leopard, "points": 20 })),
      turn("Dave", &w.leopard_hands, None)
    ]
  );
  assert!(room.take(&w.stop_dave, 50, "Owner"));
  assert_eq!(room.last()[0], line("stopped", json!({})));
  assert!(!room.games.playing(7));
}

#[test]
fn a_start_is_refused_where_no_word_book_is_installed() {
  let book = tempfile::tempdir().expect("a scratch directory");
  let absent = book.path().join("absent.sqlite");
  let room = Room::on(absent.clone(), book);
  let refused = room.games.start(start(7, None, None)).expect_err("no book");

  assert_eq!(refused.code, "no-book");
  assert!(refused.line.contains(&absent.display().to_string()), "{}", refused.line);
}
