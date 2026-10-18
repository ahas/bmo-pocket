//! The service at its boundary (docs/api.md): its control routes behind their token, a game opened and stopped
//! through them by the command line and by the tool, a step's message rendered by the web unit and said through
//! the receiver, and a room's message off the receiver's event stream handed to the games. Every other unit
//! stands in on a loopback port of its own.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::{Path as Route, State};
use axum::http::{StatusCode, Uri};
use axum::routing::{get, post};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use word_chain::config::{Config, Peer};
use word_chain::lines::{Line, Message};
use word_chain::rooms::{Parts, Say, Tuning, WordChainRooms, WordChainStart};
use word_chain::serve::{on_frame, speak};
use word_chain::{SqliteWordBook, control};

// What a stand-in was asked: each request by its path and body.
type Asked = Arc<Mutex<Vec<(String, Value)>>>;

async fn listen(app: Router) -> String {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap().to_string();

  tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
  address
}

// A stand-in answering every POST with `answer` for its path, keeping what it was asked.
async fn stand_in(answer: fn(&str, &Value) -> (StatusCode, Value)) -> (String, Asked) {
  let asked: Asked = Arc::default();
  let app = Router::new()
    .fallback(post(move |State(asked): State<Asked>, uri: Uri, Json(body): Json<Value>| async move {
      asked.lock().unwrap().push((uri.path().to_owned(), body.clone()));

      let (status, said) = answer(uri.path(), &body);

      (status, Json(said))
    }))
    .with_state(asked.clone());

  (listen(app).await, asked)
}

fn config(data: &Path, control: &str, receiver: &str, web: &str) -> Config {
  let config = Config {
    listen: control.to_owned(),
    token: "control-token".to_owned(),
    data_root: data.to_owned(),
    language: "ko".to_owned(),
    agent: "agent-a".to_owned(),
    robot_name: "Robot".to_owned(),
    receiver: Peer {
      base: format!("http://{receiver}"),
      token: "receiver-token".to_owned(),
    },
    web: Peer {
      base: format!("http://{web}"),
      token: "web-token".to_owned(),
    },
  };
  let file = Config::deployed(data);

  std::fs::create_dir_all(file.parent().unwrap()).unwrap();
  std::fs::write(
    &file,
    format!(
      "listen = {:?}\ntoken = {:?}\ndata_root = {:?}\nlanguage = \"ko\"\nagent = \"agent-a\"\nrobot_name = \"Robot\"\n\n[receiver]\nbase = {:?}\ntoken = \"receiver-token\"\n\n[web]\nbase = {:?}\ntoken = \"web-token\"\n",
      config.listen,
      config.token,
      data.display().to_string(),
      config.receiver.base,
      config.web.base
    ),
  )
  .unwrap();

  config
}

// Games over a book of two words, their messages kept.
fn rooms(data: &Path) -> (WordChainRooms, Arc<Mutex<Vec<Message>>>) {
  let book = word_chain::install::book_path(data);
  let words: Value = toml::from_str::<toml::Table>(include_str!("fixtures/rooms.toml"))
    .map(|table| serde_json::to_value(table).unwrap())
    .unwrap();

  std::fs::create_dir_all(book.parent().unwrap()).unwrap();
  SqliteWordBook::write_file(
    &book,
    &[
      (words["words"]["train"].as_str().unwrap(), false),
      (words["words"]["ticket"].as_str().unwrap(), false),
    ],
    &[],
  )
  .unwrap();

  let sent: Arc<Mutex<Vec<Message>>> = Arc::default();
  let kept = sent.clone();
  let say: Say = Arc::new(move |message| kept.lock().unwrap().push(message));

  (
    WordChainRooms::of(Parts {
      book_path: book,
      say,
      clock: Arc::new(word_chain::rooms::TaskClock),
      tuning: Tuning::default(),
      robot_name: "Robot".to_owned(),
    }),
    sent,
  )
}

fn start(chat_id: i64) -> WordChainStart {
  WordChainStart {
    chat_id,
    starter: None,
    robot: None,
    turn_seconds: None,
    loanwords: true,
    manner: false,
    opening: None,
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_control_routes_open_and_stop_a_game_behind_their_token_and_refuse_with_a_code() {
  let data = tempfile::tempdir().unwrap();
  let (games, _) = rooms(data.path());
  let address = listen(control::routes(games.clone(), "control-token".to_owned())).await;
  let client = reqwest::Client::new();
  let ask = |path: &str, token: &str, body: Value| {
    client.post(format!("http://{address}{path}")).bearer_auth(token).json(&body).send()
  };

  assert_eq!(ask("/word-chain/start", "wrong", json!({ "chatId": 7 })).await.unwrap().status(), 401);

  let opened = ask("/word-chain/start", "control-token", json!({ "chatId": 7, "robot": null }))
    .await
    .unwrap();

  assert_eq!(opened.status(), 200);
  assert_eq!(opened.json::<Value>().await.unwrap()["code"], "opened");
  assert!(games.playing(7));

  let malformed = ask("/word-chain/start", "control-token", json!({ "robot": "expert" })).await.unwrap();

  assert_eq!(
    (malformed.status(), malformed.json::<Value>().await.unwrap()["code"].clone()),
    (StatusCode::BAD_REQUEST, json!("bad-request"))
  );

  let stopped = ask("/word-chain/stop", "control-token", json!({ "chatId": 7 })).await.unwrap();

  assert_eq!(stopped.json::<Value>().await.unwrap()["code"], "stopped");

  let none = ask("/word-chain/stop", "control-token", json!({ "chatId": 7 })).await.unwrap();

  assert_eq!(
    (none.status(), none.json::<Value>().await.unwrap()["code"].clone()),
    (StatusCode::NOT_FOUND, json!("none"))
  );
  assert_eq!(client.get(format!("http://{address}/health")).send().await.unwrap().status(), 200);
}

fn answered(path: &str, _: &Value) -> (StatusCode, Value) {
  (StatusCode::OK, json!({ "ok": true, "code": "done", "line": format!("{path} done") }))
}

fn binary(data: &Path, args: &[&str], input: Option<&str>) -> (i32, String, String) {
  let mut command = Command::new(env!("CARGO_BIN_EXE_bmo-word-chain"));

  command.arg("--data-root").arg(data).args(args);

  let done = match input {
    Some(input) => {
      use std::io::Write;

      let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

      child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
      child.wait_with_output().unwrap()
    }
    None => command.output().unwrap(),
  };

  (
    done.status.code().unwrap_or(-1),
    String::from_utf8_lossy(&done.stdout).into_owned(),
    String::from_utf8_lossy(&done.stderr).into_owned(),
  )
}

// The binary is waited on from a thread of its own, so the stand-in answers it meanwhile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_game_is_opened_and_stopped_by_the_command_line_and_by_the_tool_through_the_control_routes() {
  let data = tempfile::tempdir().unwrap();
  let (control, asked) = stand_in(answered).await;

  config(data.path(), &control, "127.0.0.1:9", "127.0.0.1:9");

  let path = data.path().to_owned();
  let ran = tokio::task::spawn_blocking(move || {
    let opened = binary(
      &path,
      &["start", "--chat", "-100", "--starter", "55", "--starter-name", "Bob", "--robot", "off", "--seconds", "30", "--no-loanwords"],
      None,
    );
    let stopped = binary(&path, &["stop", "--chat", "-100", "--player", "55"], None);
    let refused = binary(&path, &["start", "--chat", "-100", "--robot", "expert"], None);
    let session = json!({ "line": "chat", "capabilities": ["room.tools"], "dataRoot": path, "runVariables": { "BMO_TURN_CHAT": "-7" } });
    let call = |entry: &str, arguments: Value| {
      json!({ "protocol": 1, "kind": "call", "entry": entry, "arguments": arguments, "session": session }).to_string()
    };
    let tool_started = binary(&path, &["tool"], Some(&call("bmo/word_chain_start", json!({ "starter": 5, "starter_name": "Ann", "word": "w" }))));
    let tool_stopped = binary(&path, &["tool"], Some(&call("bmo/word_chain_stop", json!({ "chat": -8 }))));
    let tool_unknown = binary(&path, &["tool"], Some(&call("bmo/word_chain_other", json!({}))));

    (opened, stopped, refused, tool_started, tool_stopped, tool_unknown)
  })
  .await
  .unwrap();
  let (opened, stopped, refused, tool_started, tool_stopped, tool_unknown) = ran;

  assert_eq!((opened.0, opened.1.as_str()), (0, "/word-chain/start done\n"), "{}", opened.2);
  assert_eq!((stopped.0, stopped.1.as_str()), (0, "/word-chain/stop done\n"));
  assert_eq!(refused.0, 1);
  assert!(refused.2.contains("bmo takes off, easy, normal or hard"), "{}", refused.2);

  let answer =
    |ran: &(i32, String, String)| -> Value { serde_json::from_str(&ran.1).unwrap_or(Value::Null) };

  assert_eq!(answer(&tool_started)["content"][0]["text"], "/word-chain/start done");
  assert_eq!(answer(&tool_stopped)["isError"], false);
  assert_eq!(answer(&tool_unknown)["isError"], true);
  assert_eq!(
    asked.lock().unwrap().clone(),
    [
      (
        "/word-chain/start".to_owned(),
        json!({ "chatId": -100, "starterId": 55, "starterName": "Bob", "robot": null, "turnSeconds": 30, "loanwords": false, "manner": false })
      ),
      ("/word-chain/stop".to_owned(), json!({ "chatId": -100, "playerId": 55 })),
      (
        "/word-chain/start".to_owned(),
        json!({ "chatId": -7, "starterId": 5, "starterName": "Ann", "robot": "normal", "opening": "w", "loanwords": true, "manner": false })
      ),
      ("/word-chain/stop".to_owned(), json!({ "chatId": -8 })),
    ]
  );
}

// The web unit standing in: it renders `word-chain.joined` and refuses every other code it does not hold.
fn renders(_: &str, asked: &Value) -> (StatusCode, Value) {
  match asked["code"].as_str() {
    Some("word-chain.joined") => (
      StatusCode::OK,
      json!({ "text": format!("{} joined", asked["name"].as_str().unwrap_or_default()) }),
    ),
    _ => (StatusCode::NOT_FOUND, json!({ "error": "No such code.", "code": "unknown_code" })),
  }
}

fn says(_: &str, _: &Value) -> (StatusCode, Value) {
  (StatusCode::OK, json!({ "lines": [] }))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_steps_message_is_rendered_line_by_line_by_the_web_unit_and_said_through_the_receiver_as_the_agent()
 {
  let data = tempfile::tempdir().unwrap();
  let (web, rendered) = stand_in(renders).await;
  let (receiver, sent) = stand_in(says).await;
  let config = config(data.path(), "127.0.0.1:9", &receiver, &web);
  let message = Message {
    chat_id: 7,
    lines: vec![Line::of("joined", json!({ "name": "Erin", "count": 1 })), Line::of("folded", json!({}))],
  };

  speak(&reqwest::Client::new(), &config, &message).await;

  assert_eq!(
    rendered.lock().unwrap()[0].1,
    json!({ "code": "word-chain.joined", "language": "ko", "name": "Erin", "count": 1 })
  );
  // A code the web unit renders no line for goes out as its code and fields, so the room is never silent.
  assert_eq!(
    sent.lock().unwrap().clone(),
    [(
      "/sends".to_owned(),
      json!({
        "asker": { "chat": 7, "agent": "agent-a", "job": false },
        "tool": "say",
        "input": { "chat": 7, "text": "Erin joined\nword-chain.folded {}" },
      })
    )]
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rooms_message_off_the_event_stream_is_read_and_handed_to_the_games_and_a_removed_room_forgotten()
{
  let data = tempfile::tempdir().unwrap();
  let (games, sent) = rooms(data.path());
  let join: Value =
    serde_json::to_value(toml::from_str::<toml::Table>(include_str!("fixtures/rooms.toml")).unwrap())
      .unwrap()["words"]["join"]
      .clone();
  let receiver = listen(
    Router::new()
      .route(
        "/rooms/{chat}/messages/{message}",
        get(move |Route((chat, message)): Route<(i64, i64)>| {
          let join = join.clone();

          async move { Json(json!({ "sender": message, "sender_name": format!("person-{message}"), "sent_at": 1, "text": join, "chat": chat })) }
        }),
      )
      .route("/messages/sender-is-owner", post(|| async { Json(json!({ "owner": false })) })),
  )
  .await;
  let peer = Peer {
    base: format!("http://{receiver}"),
    token: "receiver-token".to_owned(),
  };
  let client = reqwest::Client::new();
  let mut cursor = (None, 0);

  on_frame(&client, &peer, &games, &mut cursor, Some("run"), None, r#"{ "run": "run-1" }"#).await;
  // A message of a room no game stands in is not read.
  on_frame(
    &client,
    &peer,
    &games,
    &mut cursor,
    Some("bot"),
    Some("4"),
    r#"{ "type": "message", "chatId": 7, "messageId": 10 }"#,
  )
  .await;
  assert_eq!(cursor, (Some("run-1".to_owned()), 4));
  assert!(games.start(start(7)).is_ok());

  let before = sent.lock().unwrap().len();

  on_frame(
    &client,
    &peer,
    &games,
    &mut cursor,
    Some("bot"),
    Some("5"),
    r#"{ "type": "message", "chatId": 7, "messageId": 11 }"#,
  )
  .await;

  let said = sent.lock().unwrap()[before..].to_vec();

  assert_eq!(said[0].lines, [Line::of("joined", json!({ "name": "person-11", "count": 1 }))]);

  // A stream of another run starts the cursor again, and a removed room's games are let go.
  on_frame(&client, &peer, &games, &mut cursor, Some("run"), None, r#"{ "run": "run-2" }"#).await;
  assert_eq!(cursor, (Some("run-2".to_owned()), 0));
  on_frame(
    &client,
    &peer,
    &games,
    &mut cursor,
    Some("bot"),
    Some("1"),
    r#"{ "type": "room-removed", "chatId": 7, "messageId": null }"#,
  )
  .await;
  assert!(!games.playing(7));
}
