//! The service's control routes (docs/api.md, 「The control routes」): a game opened in a room, and the game of
//! the person who asked stopped. The games stand in the service's process, where the rooms' messages are read, so
//! the tool and the command line reach them here.

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Map, Value, json};

use crate::game::Difficulty;
use crate::rooms::{Answer, Starter, WordChainRooms, WordChainStart};

// The largest number told apart without losing a digit, as a JavaScript number holds a whole number.
const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

// A whole number a body carries, as JavaScript takes one: a number with nothing after its point, no larger than it
// can tell apart.
fn whole(value: Option<&Value>) -> Option<i64> {
  let number = value?;
  let whole = number.as_i64().or_else(|| {
    number
      .as_f64()
      .filter(|float| float.fract() == 0.0 && float.abs() <= MAX_SAFE_INTEGER as f64)
      .map(|float| float as i64)
  })?;

  (whole.abs() <= MAX_SAFE_INTEGER).then_some(whole)
}

// Words a body carries with their edges trimmed, or nothing where it carries none.
fn words(value: Option<&Value>) -> Option<String> {
  value
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|text| !text.is_empty())
    .map(str::to_owned)
}

fn difficulty(value: Option<&Value>) -> Result<Option<Difficulty>, ()> {
  match value {
    Some(Value::Null) => Ok(None),
    Some(Value::String(named)) => match named.as_str() {
      "easy" => Ok(Some(Difficulty::Easy)),
      "normal" => Ok(Some(Difficulty::Normal)),
      "hard" => Ok(Some(Difficulty::Hard)),
      _ => Err(()),
    },
    _ => Err(()),
  }
}

/// A start as the tool sends it, read into what the games take; a sentence where it is malformed.
pub fn read_start(body: &Map<String, Value>) -> Result<WordChainStart, &'static str> {
  let chat_id = whole(body.get("chatId")).ok_or("chatId (a whole number) is required")?;
  let robot =
    difficulty(body.get("robot")).map_err(|()| "robot takes null or one of easy, normal, hard")?;
  let starter_name = words(body.get("starterName"));
  let starter = match body.get("starterId") {
    None | Some(Value::Null) => None,
    Some(id) => match (whole(Some(id)), &starter_name) {
      (Some(id), Some(name)) => Some(Starter {
        id,
        name: name.clone(),
      }),
      _ => return Err("starterId takes a whole number, and with it starterName"),
    },
  };

  Ok(WordChainStart {
    chat_id,
    starter,
    robot,
    turn_seconds: body.get("turnSeconds").and_then(Value::as_f64),
    loanwords: body.get("loanwords") != Some(&Value::Bool(false)),
    manner: body.get("manner") == Some(&Value::Bool(true)),
    opening: words(body.get("opening")),
  })
}

fn refused(status: StatusCode, code: &str, error: &str) -> Response {
  (status, Json(json!({ "ok": false, "code": code, "error": error }))).into_response()
}

fn answered(done: Result<Answer, Answer>, refusal: StatusCode) -> Response {
  match done {
    Ok(answer) => Json(json!({ "ok": true, "code": answer.code, "line": answer.line })).into_response(),
    Err(answer) => refused(refusal, answer.code, &answer.line),
  }
}

fn fields(body: Option<Json<Value>>) -> Map<String, Value> {
  match body {
    Some(Json(Value::Object(fields))) => fields,
    _ => Map::new(),
  }
}

async fn start(State(rooms): State<WordChainRooms>, body: Option<Json<Value>>) -> Response {
  match read_start(&fields(body)) {
    Ok(start) => answered(rooms.start(start), StatusCode::CONFLICT),
    Err(why) => refused(StatusCode::BAD_REQUEST, "bad-request", why),
  }
}

async fn stop(State(rooms): State<WordChainRooms>, body: Option<Json<Value>>) -> Response {
  let body = fields(body);
  let Some(chat_id) = whole(body.get("chatId")) else {
    return refused(StatusCode::BAD_REQUEST, "bad-request", "chatId (a whole number) is required");
  };
  let player = match body.get("playerId") {
    None | Some(Value::Null) => None,
    named => match whole(named) {
      Some(player) => Some(player),
      None => return refused(StatusCode::BAD_REQUEST, "bad-request", "playerId takes a whole number"),
    },
  };

  answered(rooms.stop(chat_id, player), StatusCode::NOT_FOUND)
}

async fn health() -> Json<Value> {
  Json(json!({ "ok": true }))
}

/// The control routes, every one but `GET /health` behind the bearer token.
pub fn routes(rooms: WordChainRooms, token: String) -> Router {
  let held = Router::new()
    .route("/word-chain/start", post(start))
    .route("/word-chain/stop", post(stop))
    .with_state(rooms)
    .layer(middleware::from_fn(move |request: Request, next: Next| {
      let token = token.clone();

      async move {
        let given = request.headers().get(header::AUTHORIZATION).and_then(|value| value.to_str().ok());

        if given != Some(&format!("Bearer {token}")) {
          return refused(StatusCode::UNAUTHORIZED, "unauthorized", "the bearer token is missing or wrong");
        }

        next.run(request).await
      }
    }));

  Router::new().route("/health", get(health)).merge(held)
}
