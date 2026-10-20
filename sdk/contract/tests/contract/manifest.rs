use bmo_tool::manifest::{MANIFEST, Manifest};
use bmo_tool::{Answer, Bundle, Hints, Served, Session, Tool};
use serde::Deserialize;

const MANIFEST_TEXT: &str = r#"
[tool]
name = "echo"
protocol = 1
language = "python"
run = ["python3", "echo.py"]
timeout = 30

[[entry]]
namespace = "garden"
name = "echo_back"
description = "Says the text back."
input = "echo.json"
capabilities = ["graph.read"]
hints = { read_only = true }

[entry.bundle."garden.brief"]
shown = ["text"]

[entry.listing]
line = "echo_back: says it back"
order = 2

[card]
service = "echo"
"#;

#[test]
fn a_manifest_reads_with_its_entries_bundles_listing_and_card() {
  let manifest = Manifest::parse(MANIFEST_TEXT, "echo").unwrap();
  let entry = &manifest.entries[0];

  assert_eq!(entry.full_name(), "garden/echo_back");
  assert!(Hints::from(entry.hints).read_only);
  assert_eq!(entry.bundle["garden.brief"].shown, ["text"]);
  assert_eq!(entry.listing.as_ref().unwrap().order, 2);
  assert_eq!(manifest.card.as_ref().unwrap()["service"].as_str(), Some("echo"));
  assert_eq!(manifest.named_files(), ["echo.json", "tool.toml"]);
  assert!(!manifest.typescript());
}

#[test]
fn a_manifest_breaking_a_rule_is_refused_with_why() {
  let refused = |text: &str, folder: &str| Manifest::parse(text, folder).unwrap_err();

  assert_eq!(refused(MANIFEST_TEXT, "other"), "its name `echo` is not its folder's, `other`");
  assert!(refused(&MANIFEST_TEXT.replace("protocol = 1", "protocol = 2"), "echo").contains("protocol 2"));
  assert!(refused(&MANIFEST_TEXT.replace("timeout = 30\n", ""), "echo").contains("timeout"));
  assert!(refused(&MANIFEST_TEXT.replace("language", "speech"), "echo").contains("speech"));
  assert!(
    refused(&MANIFEST_TEXT.replace(r#"capabilities = ["graph.read"]"#, ""), "echo")
      .contains("asks for no capability")
  );
  assert!(refused(&MANIFEST_TEXT.replace("echo.json", "../echo.json"), "echo").contains("beside"));
  assert!(
    refused(&MANIFEST_TEXT.replace("hints =", "unbounded = true\ntimeout = 5\nhints ="), "echo")
      .contains("unbounded and has a timeout")
  );
  assert!(refused(&MANIFEST_TEXT.replace(r#""garden""#, r#""toolbook""#), "echo").contains("namespace"));
  assert_eq!(
    refused(&format!("{MANIFEST_TEXT}\n[helper]\ndescription = \"x\"\n"), "echo"),
    "a shared helper's manifest has entries"
  );
  assert_eq!(
    refused(&MANIFEST_TEXT.replace("timeout = 30\n", "timeout = 30\nafter_deploy = []\n"), "echo"),
    "its after-deploy command names nothing to run"
  );
}

#[test]
fn a_tool_may_name_what_is_run_once_it_is_placed() {
  let declared =
    MANIFEST_TEXT.replace("timeout = 30\n", "timeout = 30\nafter_deploy = [\"bun\", \"sync.ts\"]\n");

  assert_eq!(
    Manifest::parse(&declared, "echo").unwrap().tool.after_deploy.unwrap(),
    ["bun", "sync.ts"]
  );
  assert!(Manifest::parse(MANIFEST_TEXT, "echo").unwrap().tool.after_deploy.is_none());
}

#[test]
fn a_helper_has_no_entries_and_a_system_entry_needs_no_capability() {
  let helper = "[tool]\nname = \"web\"\nprotocol = 1\nrun = [\"web-helper\"]\ntimeout = 5\n\n[helper]\ndescription = \"A record's web session.\"\n";

  assert!(Manifest::parse(helper, "web").unwrap().helper.is_some());

  let system = MANIFEST_TEXT.replace(r#"capabilities = ["graph.read"]"#, "system = true");

  assert!(Manifest::parse(&system, "echo").unwrap().entries[0].system);
}

#[test]
fn a_tool_serving_resources_alone_needs_no_entry_and_one_serving_nothing_is_refused() {
  let bare = "[tool]\nname = \"papers\"\nprotocol = 1\nrun = [\"papers\"]\ntimeout = 5\n";
  let served = format!("{bare}\n[resources]\nnamespace = \"papers\"\ncapabilities = [\"papers.read\"]\n");

  assert_eq!(Manifest::parse(&served, "papers").unwrap().resources.unwrap().namespace, "papers");
  assert_eq!(Manifest::parse(bare, "papers").unwrap_err(), "it has neither an entry nor resources");
}

#[test]
fn the_schemas_are_read_from_beside_the_manifest() {
  let dir = tempfile::tempdir().unwrap();

  std::fs::write(dir.path().join(MANIFEST), MANIFEST_TEXT).unwrap();
  assert!(Manifest::read(dir.path(), "echo").unwrap_err().contains("echo.json"));

  std::fs::write(dir.path().join("echo.json"), r#"{ "type": "object" }"#).unwrap();
  assert_eq!(Manifest::read(dir.path(), "echo").unwrap().entries[0].input_schema["type"], "object");
}

struct EchoBack;

#[derive(Deserialize, schemars::JsonSchema)]
struct EchoInput {
  /// What is said back.
  #[allow(dead_code)]
  text: String,
}

impl Tool for EchoBack {
  const NAME: &'static str = "echo_back";
  const NAMESPACE: &'static str = "garden";
  const DESCRIPTION: &'static str = "Says the text back.";
  const HINTS: Hints = Hints::READ_ONLY;
  const CAPABILITIES: &'static [&'static str] = &["graph.read"];
  const BUNDLE: Option<Bundle> = Some(Bundle {
    capability: "garden.brief",
    shown: &["text"],
    required: None,
  });

  type Input = EchoInput;

  async fn call(&self, _: &Session, _: EchoInput) -> Answer {
    Answer::text("")
  }
}

#[test]
fn a_manifest_is_held_to_the_rust_tools_answering_its_entries() {
  let dir = tempfile::tempdir().unwrap();
  let schema = EchoBack.input_schema().to_string();

  std::fs::write(dir.path().join(MANIFEST), MANIFEST_TEXT).unwrap();
  std::fs::write(dir.path().join("echo.json"), &schema).unwrap();
  assert_eq!(
    Manifest::read(dir.path(), "echo").unwrap().differences(&[&EchoBack]),
    Vec::<String>::new()
  );

  std::fs::write(dir.path().join(MANIFEST), MANIFEST_TEXT.replace("Says the text back.", "Says it."))
    .unwrap();
  std::fs::write(dir.path().join("echo.json"), r#"{ "type": "object" }"#).unwrap();

  let differ = Manifest::read(dir.path(), "echo").unwrap().differences(&[&EchoBack]);

  assert_eq!(differ[0], "garden/echo_back: its description differs from its tool's");
  assert_eq!(
    differ[1],
    format!("garden/echo_back: echo.json is not its input type's schema, which is {schema}")
  );
  assert_eq!(differ.len(), 2);
  assert_eq!(
    Manifest::read(dir.path(), "echo").unwrap().differences(&[]),
    ["garden/echo_back: no tool answers it"]
  );
}

#[test]
fn an_entry_claims_its_hosts_and_their_subdomains_and_nothing_else() {
  let manifest = Manifest::parse(
    "[tool]\nname = \"shop\"\nprotocol = 1\nrun = [\"shop\"]\ntimeout = 5\n\n[[entry]]\nnamespace = \"web\"\nname = \"shop_price\"\ndescription = \"A price.\"\ninput = \"in.json\"\ncapabilities = [\"web.sites\"]\n\n[entry.claims]\nhosts = [\"shop.example\"]\n",
    "shop",
  )
  .unwrap();
  let entry = &manifest.entries[0];

  assert!(entry.claims("https://search.prices.shop.example/x"));
  assert!(entry.claims("https://SHOP.example/"));
  assert!(!entry.claims("https://notshop.example/"));
  assert!(!entry.claims("not an address"));
}
