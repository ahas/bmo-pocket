use std::collections::BTreeMap;
use std::path::PathBuf;

use bmo_tool::protocol::{Asked, CallBack, Handed, Sockets, answer_call, answer_read};
use bmo_tool::{Answer, Hints, Pending, Resources, Session, Tool};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

struct Shout;

#[derive(Deserialize, JsonSchema)]
struct ShoutInput {
  text: String,
}

impl Tool for Shout {
  const NAME: &'static str = "shout";
  const NAMESPACE: &'static str = "worker";
  const DESCRIPTION: &'static str = "Says the text louder.";
  const HINTS: Hints = Hints::READ_ONLY;
  const CAPABILITIES: &'static [&'static str] = &["graph.read"];

  type Input = ShoutInput;

  fn claim(address: &str) -> Option<Value> {
    address.strip_prefix("https://shout.test").map(|path| json!({ "text": path }))
  }

  async fn call(&self, session: &Session, input: ShoutInput) -> Answer {
    Answer::text(format!("{}:{}", session.line, input.text.to_uppercase()))
  }
}

fn handed(asked: Value) -> Handed {
  let mut said = json!({ "protocol": 1, "session": { "line": "worker", "capabilities": [], "dataRoot": "/d", "archive": "/d/bmo.db" }, "callback": "/run/c.sock" });

  said.as_object_mut().unwrap().extend(asked.as_object().unwrap().clone());
  serde_json::from_value(said).unwrap()
}

#[test]
fn what_a_call_is_handed_reads_as_the_host_writes_it() {
  let read = handed(json!({ "kind": "call", "entry": "worker/shout", "arguments": { "text": "a" } }));

  assert_eq!(
    read.asked,
    Asked::Call {
      entry: "worker/shout".into(),
      arguments: json!({ "text": "a" })
    }
  );
  assert_eq!(serde_json::to_value(&read).unwrap()["kind"], "call");
  assert_eq!(
    serde_json::from_value::<CallBack>(json!({ "kind": "claim", "address": "https://x" })).unwrap(),
    CallBack::Claim {
      address: "https://x".into()
    }
  );
}

#[tokio::test]
async fn a_rust_tool_answers_the_entry_it_is_asked_for_and_says_which_it_does_not_hold() {
  let asked = handed(json!({ "kind": "call", "entry": "worker/shout", "arguments": { "text": "a" } }));

  assert_eq!(answer_call(&[&Shout], asked).await, Ok(Answer::text("worker:A")));

  let other = handed(json!({ "kind": "call", "entry": "worker/whisper", "arguments": {} }));

  assert!(answer_call(&[&Shout], other).await.unwrap().plain().contains("no entry worker/whisper"));
}

// Asks the call-back socket it was handed which entry claims the address, and answers what it was told.
struct Ask;

#[derive(Deserialize, JsonSchema)]
struct AskInput {
  address: String,
}

impl Tool for Ask {
  const NAME: &'static str = "ask";
  const NAMESPACE: &'static str = "worker";
  const DESCRIPTION: &'static str = "Asks the host back.";
  const HINTS: Hints = Hints::READ_ONLY;
  const CAPABILITIES: &'static [&'static str] = &["graph.read"];

  type Input = AskInput;

  async fn call(&self, session: &Session, input: AskInput) -> Answer {
    let sockets = Sockets::of(session);

    match sockets
      .call_back(&CallBack::Claim {
        address: input.address,
      })
      .await
    {
      Ok(said) => Answer::text(format!("{said} {}", sockets.helper("web").unwrap().display())),
      Err(why) => Answer::failure(why),
    }
  }
}

#[tokio::test]
async fn a_tool_reaches_the_sockets_its_call_was_handed_one_line_each_way() {
  let dir = tempfile::tempdir().unwrap();
  let socket = dir.path().join("c.sock");
  let listener = tokio::net::UnixListener::bind(&socket).unwrap();

  tokio::spawn(async move {
    let (stream, _) = listener.accept().await.unwrap();
    let (reading, mut writing) = stream.into_split();
    let mut line = String::new();

    BufReader::new(reading).read_line(&mut line).await.unwrap();

    let asked: CallBack = serde_json::from_str(&line).unwrap();
    let CallBack::Claim { address } = asked else { panic!("asked {asked:?}") };

    writing
      .write_all(format!("{}\n", json!({ "entry": address })).as_bytes())
      .await
      .unwrap();
  });

  let mut asked =
    handed(json!({ "kind": "call", "entry": "worker/ask", "arguments": { "address": "https://x" } }));

  asked.callback = Some(socket);
  asked.shared = BTreeMap::from([("web".to_owned(), PathBuf::from("/run/web.sock"))]);
  assert_eq!(
    answer_call(&[&Ask], asked).await,
    Ok(Answer::text(r#"{"entry":"https://x"} /run/web.sock"#))
  );

  // A socket nobody listens on is said to be out of reach rather than waited on.
  let gone =
    handed(json!({ "kind": "call", "entry": "worker/ask", "arguments": { "address": "https://x" } }));

  assert!(answer_call(&[&Ask], gone).await.unwrap().plain().contains("could not be reached"));
}

#[tokio::test]
async fn a_call_read_from_its_input_is_answered_on_its_output_as_a_tool_result() {
  let asked = |arguments: Value| {
    serde_json::to_vec(&handed(json!({ "kind": "call", "entry": "worker/shout", "arguments": arguments })))
      .unwrap()
  };
  let mut written = Vec::new();

  answer_read(&[&Shout], None, asked(json!({ "text": "a" })).as_slice(), &mut written)
    .await
    .unwrap();
  assert_eq!(serde_json::from_slice::<Answer>(&written).unwrap(), Answer::text("worker:A"));

  written.clear();
  answer_read(&[&Shout], None, asked(json!({ "words": "a" })).as_slice(), &mut written)
    .await
    .unwrap();
  assert!(
    serde_json::from_slice::<Answer>(&written)
      .unwrap()
      .plain()
      .contains("invalid arguments: shout")
  );

  assert!(
    answer_read(&[&Shout], None, b"{".as_slice(), &mut written)
      .await
      .unwrap_err()
      .contains("does not read")
  );
}

#[tokio::test]
async fn a_claim_request_is_answered_with_the_arguments_the_entry_reads_the_address_with() {
  let claimed = |entry: &str, address: &str| {
    let asked =
      serde_json::to_vec(&handed(json!({ "kind": "claim", "entry": entry, "address": address }))).unwrap();

    async move {
      let mut written = Vec::new();

      answer_read(&[&Shout], None, asked.as_slice(), &mut written).await.unwrap();
      serde_json::from_slice::<Value>(&written).unwrap()
    }
  };

  assert_eq!(
    claimed("worker/shout", "https://shout.test/hey").await,
    json!({ "arguments": { "text": "/hey" } })
  );
  assert_eq!(claimed("worker/shout", "https://other.test/").await, json!({ "arguments": null }));
  assert_eq!(claimed("worker/whisper", "https://shout.test/").await, json!({ "arguments": null }));
}

// One document, `paper://one`, read as the line of the session that reads it.
struct Papers;

impl Resources for Papers {
  fn list<'a>(&'a self, _: &'a Session) -> Pending<'a, Vec<Value>> {
    Box::pin(async { vec![json!({ "uri": "paper://one", "name": "one" })] })
  }

  fn read<'a>(&'a self, session: &'a Session, uri: &'a str) -> Pending<'a, Result<Value, String>> {
    Box::pin(async move {
      match uri {
        "paper://one" => Ok(json!({ "contents": [{ "uri": uri, "text": session.line }] })),
        _ => Err(format!("no paper {uri}")),
      }
    })
  }
}

#[tokio::test]
async fn a_resource_request_is_answered_by_the_resources_the_executable_serves_and_a_refused_read_says_why()
{
  let asked = |resources: Option<&'static dyn Resources>, asked: Value| async move {
    let mut written = Vec::new();

    answer_read(
      &[&Shout],
      resources,
      serde_json::to_vec(&handed(asked)).unwrap().as_slice(),
      &mut written,
    )
    .await
    .unwrap();
    serde_json::from_slice::<Value>(&written).unwrap()
  };

  assert_eq!(
    asked(Some(&Papers), json!({ "kind": "resources_list" })).await,
    json!({ "resources": [{ "uri": "paper://one", "name": "one" }] })
  );
  assert_eq!(
    asked(Some(&Papers), json!({ "kind": "resources_read", "uri": "paper://one" })).await,
    json!({ "contents": [{ "uri": "paper://one", "text": "worker" }] })
  );
  assert_eq!(
    asked(Some(&Papers), json!({ "kind": "resources_read", "uri": "paper://two" })).await,
    json!({ "error": "no paper paper://two" })
  );
  assert_eq!(asked(None, json!({ "kind": "resources_list" })).await, json!({ "resources": [] }));
  assert_eq!(
    asked(None, json!({ "kind": "resources_read", "uri": "paper://one" })).await,
    json!({ "error": "this executable serves no resources" })
  );
}
