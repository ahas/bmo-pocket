//! The example tools under `examples/` (docs/authoring.md, 「The examples」): each put from its folder into a scratch
//! registry as `bmo-pocket put` puts it, its build run where it has one, and called through the host as a session
//! calls it, every language answering the same count; a put that does not allow what a tool asks for refused naming
//! it; arguments that do not fit an entry's schema refused before the tool runs; and each example's own check
//! passing in its folder.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use bmo_tool::Session;
use bmo_tool::manifest::Manifest;
use mcp_host::drain::Drain;
use mcp_host::handler::Host;
use mcp_host::install::{self, Put};
use mcp_host::placed::Placed;
use mcp_host::registry::Registry;
use serde_json::{Value, json};
use tempfile::TempDir;

/// Each example's folder, which is its tool's name, and its entry's name in the `example` namespace.
const EXAMPLES: [(&str, &str); 3] = [
  ("count-words-rust", "count_words_rust"),
  ("count-words-ts", "count_words_ts"),
  ("count-words-py", "count_words_py"),
];

/// The examples whose own check this test runs: the Rust example is a member of the workspace, whose tests run its
/// check's tests already.
const CHECKED: [&str; 2] = ["count-words-ts", "count-words-py"];

/// What the example's manifest asks a put to allow.
const CAPABILITY: &str = "example.read";

fn example(folder: &str) -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(folder)
}

struct Scratch {
  dir: TempDir,
  // Sockets are bound under a short path of their own.
  sockets: TempDir,
}

impl Scratch {
  fn new() -> Self {
    Self {
      dir: tempfile::tempdir().unwrap(),
      sockets: tempfile::Builder::new().prefix("ex").tempdir_in("/tmp").unwrap(),
    }
  }

  fn registry(&self) -> Registry {
    Registry::of(&self.dir.path().join("data"))
  }

  // A host serving the registry alone, its catalogue read once.
  fn host(&self) -> Arc<Host> {
    let placed = Placed::new(None, self.sockets.path().to_owned()).registered(self.registry());

    placed.reread();

    Arc::new(Host {
      placed: Arc::new(placed),
      pass_env: Vec::new(),
      drain: Drain::default(),
    })
  }
}

// A record using the examples' namespace and holding their capability.
fn session() -> Session {
  serde_json::from_value(json!({
    "holderName": "tester",
    "namespaces": ["example"],
    "capabilities": [CAPABILITY],
    "grants": [],
  }))
  .unwrap()
}

// The example put from its folder, allowing what is named: what the put said, or its refusal.
fn put(registry: &Registry, folder: &str, allow: &[&str]) -> Result<Vec<String>, String> {
  let asked = Put {
    tool: example(folder).display().to_string(),
    allow: allow.iter().map(|one| (*one).to_owned()).collect(),
    ..Put::default()
  };
  let mut said = Vec::new();

  install::put(registry, &asked, &mut said).map(|()| said)
}

// One call of the example's entry under the record, answered as a tool result.
async fn called(host: &Arc<Host>, entry: &str, arguments: Value) -> Value {
  let session = session();
  let found = host.find(&session, "example", entry).expect("the record holds the entry");

  host.answer(&found, &session, arguments).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn every_example_is_put_and_answers_the_same_count_through_the_host() {
  let scratch = Scratch::new();
  let registry = scratch.registry();

  for (folder, _) in EXAMPLES {
    let said = put(&registry, folder, &[CAPABILITY]).unwrap_or_else(|why| panic!("{folder}: {why}"));

    assert!(
      said.contains(&format!("{folder} asks for the capability {CAPABILITY}")),
      "{folder}: {said:?}"
    );
  }

  let host = scratch.host();

  for (folder, entry) in EXAMPLES {
    let answer = called(&host, entry, json!({ "text": "one two  three\n" })).await;

    assert_eq!(answer["isError"], false, "{folder}: {answer}");
    assert_eq!(
      answer["structuredContent"],
      json!({ "words": 3, "characters": 15 }),
      "{folder}: {answer}"
    );
  }
}

#[test]
fn an_example_put_without_its_capability_allowed_is_refused_naming_it_and_places_nothing() {
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let refused = put(&registry, "count-words-py", &[]).unwrap_err();

  assert!(refused.contains(CAPABILITY), "{refused}");
  assert!(scratch.host().find(&session(), "example", "count_words_py").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn arguments_that_do_not_fit_the_entry_s_schema_are_refused_with_it_before_the_tool_runs() {
  let scratch = Scratch::new();

  put(&scratch.registry(), "count-words-py", &[CAPABILITY]).unwrap();

  let answer = called(&scratch.host(), "count_words_py", json!({ "words": "one" })).await;
  let text = answer["content"][0]["text"].as_str().unwrap_or_default();

  assert_eq!(answer["isError"], true, "{answer}");
  assert!(text.contains("do not fit its schema"), "{text}");
  assert!(text.contains("\"text\""), "{text}");
}

#[test]
fn each_example_s_own_check_passes_in_its_folder() {
  for folder in CHECKED {
    let dir = example(folder);
    let manifest = Manifest::read(&dir, folder).unwrap_or_else(|why| panic!("{folder}: {why}"));
    let check = manifest.tool.check.unwrap_or_else(|| panic!("{folder} names no check"));
    let ran = Command::new("sh").args(["-c", &check]).current_dir(&dir).output().unwrap();

    assert!(
      ran.status.success(),
      "{folder}: {check} failed with {}:\n{}\n{}",
      ran.status,
      String::from_utf8_lossy(&ran.stdout),
      String::from_utf8_lossy(&ran.stderr),
    );
  }
}

#[test]
fn each_example_carries_the_kit_of_its_language_unchanged() {
  let root = Path::new(env!("CARGO_MANIFEST_DIR"));

  for (kit, copy) in [
    ("sdk/py-kit/src/bmo_tool.py", "examples/count-words-py/bmo_tool.py"),
    ("sdk/ts-kit/src/tool.ts", "examples/count-words-ts/tool.ts"),
  ] {
    let kit_text = std::fs::read_to_string(root.join(kit)).unwrap();
    let copy_text = std::fs::read_to_string(root.join(copy)).unwrap();

    assert!(kit_text == copy_text, "{copy} is not {kit} as it stands");
  }
}

#[test]
fn the_skill_leads_to_the_guide_with_the_verbs_that_put_list_and_take_a_tool() {
  let root = Path::new(env!("CARGO_MANIFEST_DIR"));
  let skill = std::fs::read_to_string(root.join("plugin/skills/bmo-pocket/SKILL.md")).unwrap();
  let making = skill.split("## Making a tool").nth(1).expect("the skill says how a tool is made");
  let making = making.split("\n## ").next().unwrap_or_default();

  assert!(root.join("docs/authoring.md").is_file());
  for said in [
    "`docs/authoring.md`",
    "`examples/`",
    "bmo pocket put",
    "--allow",
    "bmo pocket list",
    "bmo pocket take",
  ] {
    assert!(making.contains(said), "{said}: {making}");
  }
}
