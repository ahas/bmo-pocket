use bmo_tool::{Answer, Bundle, CallError, Held, Hints, Served, Session, Tool};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

// A tool as a tool crate writes one.
struct Echo;

#[derive(Deserialize, JsonSchema)]
struct EchoInput {
  /// What to say back.
  text: String,
  times: Option<u8>,
}

impl Tool for Echo {
  const NAME: &'static str = "echo";
  const NAMESPACE: &'static str = "worker";
  const DESCRIPTION: &'static str = "Says the text back.";
  const HINTS: Hints = Hints::READ_ONLY;
  const CAPABILITIES: &'static [&'static str] = &["graph.read"];
  const BUNDLE: Option<Bundle> = Some(Bundle {
    capability: "room.chat",
    shown: &["text"],
    required: None,
  });

  type Input = EchoInput;

  async fn call(&self, session: &Session, input: EchoInput) -> Answer {
    Answer::text(format!("{}:{}", session.line, input.text.repeat(input.times.unwrap_or(1).into())))
  }
}

fn session(capabilities: &[&str]) -> Session {
  Session {
    line: "worker".into(),
    capabilities: capabilities.iter().map(|one| (*one).to_owned()).collect(),
    ..Default::default()
  }
}

#[test]
fn a_session_holds_a_tool_whole_by_its_capabilities_or_in_its_bundle_s_form_by_the_bundle_s() {
  assert_eq!(Echo.held(&session(&["graph.read"])), Some(Held::Whole));
  assert_eq!(Echo.held(&session(&["room.chat"])), Some(Held::Bundle));
  assert_eq!(Echo.held(&session(&["queue.read"])), None);
  assert_eq!(Echo.schema_held(Held::Bundle)["properties"].as_object().unwrap().len(), 1);
}

#[test]
fn the_input_schema_is_derived_from_the_input_type() {
  let schema = Echo.input_schema();

  assert_eq!(schema["type"], "object");
  assert_eq!(schema["properties"]["text"]["type"], "string");
  assert_eq!(schema["properties"]["text"]["description"], "What to say back.");
  assert_eq!(schema["required"], json!(["text"]));
  assert!(schema.get("$schema").is_none());
}

#[tokio::test]
async fn a_call_reads_its_arguments_into_the_input_and_refuses_what_does_not_fit() {
  let asked = session(&["graph.read"]);

  assert_eq!(
    Served::call(&Echo, &asked, json!({ "text": "ga", "times": 2 })).await,
    Ok(Answer::text("worker:gaga"))
  );
  assert!(
    matches!(Served::call(&Echo, &asked, json!({ "times": 2 })).await, Err(CallError::InvalidArguments(why)) if why.starts_with("echo: "))
  );
  assert!(matches!(
    Served::call(&Echo, &asked, Value::Null).await,
    Err(CallError::InvalidArguments(_))
  ));
}
