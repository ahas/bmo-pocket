//! The resident service (docs/word-chain.md, 「The service」): the games, the receiver's event stream read for the
//! rooms' messages while a game stands in them, each step's message rendered by the web unit and said through the
//! receiver, and the control routes the tool and the command line reach the games by (docs/api.md).

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{Value, json};
use sse_stream::SseStream;
use tokio::sync::mpsc;

use crate::config::{Config, Peer};
use crate::install::book_path;
use crate::lines::{Line, Message};
use crate::rooms::{RoomLine, Say, WordChainRooms};

// How long a stream that broke is waited on before it is opened again.
const REOPEN_WAIT: Duration = Duration::from_secs(1);

/// The prefix a line's code is rendered under in the web unit's packs.
pub const PACK: &str = "word-chain";

/// One line rendered by the web unit in the language the configuration names; where the web unit renders none,
/// the line goes out as its code and fields, so a room is never silent.
pub async fn render(client: &reqwest::Client, web: &Peer, language: &str, line: &Line) -> String {
  let mut asked = line.fields.clone();

  asked.insert("code".to_owned(), json!(format!("{PACK}.{}", line.code)));
  asked.insert("language".to_owned(), json!(language));

  let rendered = async {
    let reply = client
      .post(web.url("/words/render"))
      .bearer_auth(&web.token)
      .json(&asked)
      .send()
      .await
      .ok()?;

    if !reply.status().is_success() {
      return None;
    }

    reply.json::<Value>().await.ok()?.get("text")?.as_str().map(str::to_owned)
  };

  match rendered.await {
    Some(text) => text,
    None => format!("{PACK}.{} {}", line.code, Value::Object(line.fields.clone())),
  }
}

/// One message rendered line by line and said in its room through the receiver, as the configured agent.
pub async fn speak(client: &reqwest::Client, config: &Config, message: &Message) {
  let mut text = Vec::new();

  for line in &message.lines {
    text.push(render(client, &config.web, &config.language, line).await);
  }

  let asked = json!({
    "asker": { "chat": message.chat_id, "agent": config.agent, "job": false },
    "tool": "say",
    "input": { "chat": message.chat_id, "text": text.join("\n") },
  });
  let sent = client
    .post(config.receiver.url("/sends"))
    .bearer_auth(&config.receiver.token)
    .json(&asked)
    .send()
    .await;

  match sent {
    Ok(reply) if reply.status().is_success() => {}
    Ok(reply) => eprintln!("word chain: room {} was not said to: {}", message.chat_id, reply.status()),
    Err(why) => eprintln!("word chain: room {} was not said to: {why}", message.chat_id),
  }
}

/// Where the games' messages go: one task saying them in the order they were said, so no send runs inside a step
/// and no room hears two steps out of order.
pub fn speaker(client: reqwest::Client, config: Arc<Config>) -> Say {
  let (said, mut saying) = mpsc::unbounded_channel::<Message>();

  tokio::spawn(async move {
    while let Some(message) = saying.recv().await {
      speak(&client, &config, &message).await;
    }
  });

  Arc::new(move |message| {
    let _ = said.send(message);
  })
}

/// One message of a room read and handed to the games: its text and sender from the receiver, and whether its
/// sender is the owner. Read only where a game stands in the room.
pub async fn hand_on(
  client: &reqwest::Client,
  receiver: &Peer,
  rooms: &WordChainRooms,
  chat_id: i64,
  message_id: i64,
) {
  if !rooms.playing(chat_id) {
    return;
  }

  let read = async {
    let message: Value = client
      .get(receiver.url(&format!("/rooms/{chat_id}/messages/{message_id}")))
      .bearer_auth(&receiver.token)
      .send()
      .await
      .ok()?
      .error_for_status()
      .ok()?
      .json()
      .await
      .ok()?;
    let owner: Value = client
      .post(receiver.url("/messages/sender-is-owner"))
      .bearer_auth(&receiver.token)
      .json(&json!({ "chatId": chat_id, "messageId": message_id }))
      .send()
      .await
      .ok()?
      .json()
      .await
      .ok()?;

    Some((message, owner["owner"].as_bool() == Some(true)))
  };
  let Some((message, owner)) = read.await else {
    eprintln!("word chain: message {message_id} of room {chat_id} could not be read");
    return;
  };

  rooms.take(&RoomLine {
    chat_id,
    sender_id: message["sender"].as_i64(),
    sender_name: message["sender_name"].as_str().unwrap_or_default(),
    text: message["text"].as_str().unwrap_or_default(),
    owner,
  });
}

/// What one frame of the event stream asks of the games, the cursor moved past it: a message handed on, a room
/// removed forgotten. A frame of another run starts the cursor again.
pub async fn on_frame(
  client: &reqwest::Client,
  receiver: &Peer,
  rooms: &WordChainRooms,
  cursor: &mut (Option<String>, i64),
  kind: Option<&str>,
  id: Option<&str>,
  data: &str,
) {
  let said: Value = serde_json::from_str(data).unwrap_or(Value::Null);

  match kind {
    Some("run") => {
      let run = said["run"].as_str().map(str::to_owned);

      if run != cursor.0 {
        *cursor = (run, 0);
      }
    }
    Some("bot") => {
      if let Some(id) = id.and_then(|id| id.parse().ok()) {
        cursor.1 = id;
      }

      let chat_id = said["chatId"].as_i64().unwrap_or_default();

      match said["type"].as_str() {
        Some("message") => {
          if let Some(message_id) = said["messageId"].as_i64() {
            hand_on(client, receiver, rooms, chat_id, message_id).await;
          }
        }
        Some("room-removed") => rooms.forget(chat_id),
        _ => {}
      }
    }
    _ => {}
  }
}

/// The receiver's event stream read for as long as the service runs, opened again from where it stood when it
/// breaks.
pub async fn read_events(client: reqwest::Client, receiver: Peer, rooms: WordChainRooms) {
  let mut cursor: (Option<String>, i64) = (None, 0);

  loop {
    let mut asked = vec![("after", cursor.1.to_string())];

    if let Some(run) = &cursor.0 {
      asked.push(("run", run.clone()));
    }

    let opened = client
      .get(receiver.url("/events"))
      .bearer_auth(&receiver.token)
      .query(&asked)
      .send()
      .await;

    match opened.and_then(reqwest::Response::error_for_status) {
      Ok(stream) => {
        let mut frames = std::pin::pin!(SseStream::from_bytes_stream(stream.bytes_stream()));

        while let Some(Ok(frame)) = frames.next().await {
          let data = frame.data.unwrap_or_default();

          on_frame(
            &client,
            &receiver,
            &rooms,
            &mut cursor,
            frame.event.as_deref(),
            frame.id.as_deref(),
            &data,
          )
          .await;
        }
      }
      Err(why) => eprintln!("word chain: the receiver's event stream did not open: {why}"),
    }

    tokio::time::sleep(REOPEN_WAIT).await;
  }
}

/// The service: the games, the event stream read, and the control routes answered on the configured address until
/// the process is stopped.
pub async fn serve(config: Config) -> Result<(), String> {
  let client = reqwest::Client::new();
  let config = Arc::new(config);
  let rooms = WordChainRooms::new(
    book_path(&config.data_root),
    speaker(client.clone(), config.clone()),
    config.robot_name.clone(),
  );
  let listener = tokio::net::TcpListener::bind(&config.listen)
    .await
    .map_err(|why| format!("{} could not be bound: {why}", config.listen))?;

  tokio::spawn(read_events(client, config.receiver.clone(), rooms.clone()));
  println!("word chain listening on {}", config.listen);

  axum::serve(listener, crate::control::routes(rooms, config.token.clone()))
    .await
    .map_err(|why| why.to_string())
}
