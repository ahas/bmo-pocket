use bmo_tool::manifest::{AppPart, MANIFEST, Manifest, screen_name, screen_path};
use bmo_tool::{Answer, Bundle, Hints, Served, Session, Tool};
use serde::Deserialize;

const MANIFEST_TEXT: &str = r#"
[tool]
name = "echo"
protocol = 1
language = "python"
run = ["python3", "echo.py"]

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
  assert!(refused(&MANIFEST_TEXT.replace("language", "speech"), "echo").contains("speech"));
  assert!(
    refused(&MANIFEST_TEXT.replace(r#"capabilities = ["graph.read"]"#, ""), "echo")
      .contains("asks for no capability")
  );
  assert!(refused(&MANIFEST_TEXT.replace("echo.json", "../echo.json"), "echo").contains("beside"));
  assert!(refused(&MANIFEST_TEXT.replace(r#""garden""#, r#""toolbook""#), "echo").contains("namespace"));
  assert_eq!(
    refused(&format!("{MANIFEST_TEXT}\n[helper]\ndescription = \"x\"\n"), "echo"),
    "a shared helper's manifest has entries"
  );
  assert_eq!(
    refused(&MANIFEST_TEXT.replace("run = [", "after_deploy = []\nrun = ["), "echo"),
    "its after-deploy command names nothing to run"
  );
}

#[test]
fn a_timeout_a_manifest_still_carries_reads_and_bounds_nothing() {
  let carried = MANIFEST_TEXT
    .replace("run = [", "timeout = 30\nrun = [")
    .replace("hints =", "unbounded = true\ntimeout = 5\nhints =");

  assert!(Manifest::parse(&carried, "echo").is_ok());
  assert!(Manifest::parse(&MANIFEST_TEXT.replace("run = [", "timeout = 0\nrun = ["), "echo").is_ok());
}

#[test]
fn a_tool_may_name_what_is_run_once_it_is_placed() {
  let declared = MANIFEST_TEXT.replace("run = [", "after_deploy = [\"bun\", \"sync.ts\"]\nrun = [");

  assert_eq!(
    Manifest::parse(&declared, "echo").unwrap().tool.after_deploy.unwrap(),
    ["bun", "sync.ts"]
  );
  assert!(Manifest::parse(MANIFEST_TEXT, "echo").unwrap().tool.after_deploy.is_none());
}

#[test]
fn a_helper_has_no_entries_and_a_system_entry_needs_no_capability() {
  let helper = "[tool]\nname = \"web\"\nprotocol = 1\nrun = [\"web-helper\"]\n\n[helper]\ndescription = \"A record's web session.\"\n";

  assert!(Manifest::parse(helper, "web").unwrap().helper.is_some());

  let system = MANIFEST_TEXT.replace(r#"capabilities = ["graph.read"]"#, "system = true");

  assert!(Manifest::parse(&system, "echo").unwrap().entries[0].system);
}

#[test]
fn a_tool_serving_resources_alone_needs_no_entry_and_one_serving_nothing_is_refused() {
  let bare = "[tool]\nname = \"papers\"\nprotocol = 1\nrun = [\"papers\"]\n";
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

// The manifest naming a manual at this path.
fn with_manual(path: &str) -> String {
  MANIFEST_TEXT.replace("input = \"echo.json\"", &format!("manual = \"{path}\"\ninput = \"echo.json\""))
}

#[test]
fn a_manual_is_read_from_inside_the_tools_folder_and_named_with_its_files() {
  let dir = tempfile::tempdir().unwrap();

  std::fs::write(dir.path().join(MANIFEST), with_manual("manuals/echo_back.md")).unwrap();
  std::fs::write(dir.path().join("echo.json"), r#"{ "type": "object" }"#).unwrap();
  assert!(Manifest::read(dir.path(), "echo").unwrap_err().contains("manuals/echo_back.md"));

  std::fs::create_dir(dir.path().join("manuals")).unwrap();
  std::fs::write(dir.path().join("manuals/echo_back.md"), "# echo_back\n\nPass `text`.\n").unwrap();

  let manifest = Manifest::read(dir.path(), "echo").unwrap();

  assert_eq!(manifest.entries[0].manual.as_deref(), Some("manuals/echo_back.md"));
  assert_eq!(manifest.entries[0].manual_text.as_deref(), Some("# echo_back\n\nPass `text`.\n"));
  assert_eq!(manifest.named_files(), ["echo.json", "manuals/echo_back.md", "tool.toml"]);
}

#[test]
fn an_entry_without_a_manual_reads_with_none() {
  let dir = tempfile::tempdir().unwrap();

  std::fs::write(dir.path().join(MANIFEST), MANIFEST_TEXT).unwrap();
  std::fs::write(dir.path().join("echo.json"), r#"{ "type": "object" }"#).unwrap();

  let manifest = Manifest::read(dir.path(), "echo").unwrap();

  assert!(manifest.entries[0].manual.is_none() && manifest.entries[0].manual_text.is_none());
}

#[test]
fn a_manual_outside_the_tools_folder_is_refused() {
  for path in ["../echo_back.md", "/etc/echo_back.md", "manuals/../../echo_back.md", ""] {
    let why = Manifest::parse(&with_manual(path), "echo").unwrap_err();

    assert_eq!(
      why,
      format!("garden/echo_back: its manual `{path}` does not stand inside the tool's folder")
    );
  }

  // A link inside the folder resolving outside it is refused as the path would be.
  let outside = tempfile::tempdir().unwrap();
  let dir = tempfile::tempdir().unwrap();

  std::fs::write(outside.path().join("echo_back.md"), "elsewhere").unwrap();
  std::os::unix::fs::symlink(outside.path().join("echo_back.md"), dir.path().join("echo_back.md")).unwrap();
  std::fs::write(dir.path().join(MANIFEST), with_manual("echo_back.md")).unwrap();
  std::fs::write(dir.path().join("echo.json"), r#"{ "type": "object" }"#).unwrap();
  assert!(
    Manifest::read(dir.path(), "echo")
      .unwrap_err()
      .contains("resolves outside the tool's folder")
  );
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

#[test]
fn a_tool_names_its_app_offered_and_on_unless_it_says_otherwise() {
  let app = "[app]\nkey = \"garden-plan\"\nname = \"Garden plan\"\ndescription = \"Plans the beds.\"\n";
  let declared = Manifest::parse(&format!("{MANIFEST_TEXT}\n{app}"), "echo").unwrap().app.unwrap();

  assert_eq!(declared.key, "garden-plan");
  assert!(declared.offered && declared.on && declared.icon.is_none());
  assert!(Manifest::parse(MANIFEST_TEXT, "echo").unwrap().app.is_none());

  let closed = format!("{MANIFEST_TEXT}\n{app}offered = false\non = false\nicon = \"i-lucide-sprout\"\n");
  let closed = Manifest::parse(&closed, "echo").unwrap().app.unwrap();

  assert!(!closed.offered && !closed.on);

  let refused = |text: String| Manifest::parse(&text, "echo").unwrap_err();

  assert!(
    refused(format!("{MANIFEST_TEXT}\n{}", app.replace("garden-plan", "Garden_plan")))
      .contains("app's key")
  );
  assert!(
    refused(format!("{MANIFEST_TEXT}\n{}", app.replace("garden-plan", &"g".repeat(51))))
      .contains("app's key")
  );
  assert!(refused(format!("{MANIFEST_TEXT}\n{}", app.replace("Garden plan", " "))).contains("no name"));
  assert!(refused(format!("{MANIFEST_TEXT}\n{app}colour = \"green\"\n")).contains("colour"));

  let helper =
    "[tool]\nname = \"web\"\nprotocol = 1\nrun = [\"web-helper\"]\n\n[helper]\ndescription = \"x\"\n";

  assert_eq!(
    Manifest::parse(&format!("{helper}\n{app}"), "web").unwrap_err(),
    "a shared helper belongs to no app"
  );
}

// A screen-only app's declaration file holds the `[app]` table alone, read under the manifest's rules for it.
#[test]
fn a_declaration_file_holds_an_app_alone_under_the_manifests_rules() {
  let app = "[app]\nkey = \"garden-plan\"\nname = \"Garden plan\"\ndescription = \"Plans the beds.\"\n";
  let declared = AppPart::parse(&format!("{app}on = false\nicon = \"i-lucide-sprout\"\n")).unwrap();

  assert_eq!(declared.key, "garden-plan");
  assert!(declared.offered && !declared.on);
  assert_eq!(declared.icon.as_deref(), Some("i-lucide-sprout"));
  assert!(
    AppPart::parse(&app.replace("garden-plan", "Garden_plan"))
      .unwrap_err()
      .contains("app's key")
  );
  assert!(
    AppPart::parse(&app.replace("garden-plan", &"g".repeat(51)))
      .unwrap_err()
      .contains("app's key")
  );
  assert!(
    AppPart::parse(&app.replace("Plans the beds.", ""))
      .unwrap_err()
      .contains("no description")
  );
  assert!(AppPart::parse(&format!("{app}colour = \"green\"\n")).unwrap_err().contains("colour"));
  assert!(AppPart::parse(&format!("{MANIFEST_TEXT}\n{app}")).unwrap_err().contains("tool"));
  assert!(AppPart::parse("").unwrap_err().contains("app"));
}

#[test]
fn a_screen_carries_its_words_layer_and_index_in_the_description_and_names_an_icon_by_its_alphabet() {
  let with = |screen: &str| format!("{MANIFEST_TEXT}\n[screen]\n{screen}");
  let manifest = Manifest::parse(&with("icon = \"i-material-symbols-note\"\n"), "echo").unwrap();

  assert_eq!(manifest.screen.unwrap().icon.as_deref(), Some("i-material-symbols-note"));
  assert_eq!(
    Manifest::parse(&with(""), "echo").unwrap().description_files(),
    ["echo.json", "screen/files.json", "screen/layer.json", "screen/words.json", MANIFEST]
  );
  assert_eq!(
    Manifest::parse(MANIFEST_TEXT, "echo").unwrap().description_files(),
    ["echo.json", MANIFEST]
  );
  assert!(
    Manifest::parse(&with("icon = \"note\"\n"), "echo")
      .unwrap_err()
      .contains("no icon's name")
  );
  assert!(Manifest::parse(&with("folder = \"ui\"\n"), "echo").unwrap_err().contains("folder"));
  assert_eq!(
    (screen_name("weekly_notes"), screen_path("weekly_notes")),
    ("my-weekly-notes".to_owned(), "/my/weekly-notes".to_owned())
  );
}
