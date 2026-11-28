//! A company's catalogue (docs/catalogue.md): a share checked, declared on a stand-in account service and kept with
//! its source hash; the hash reckoned alike whatever order the files come in, and a bundle changed on disk refused;
//! the copies, the review, the approval, the withdrawal, an owner's passing and a promotion kept; a received tool
//! put with the app its pointer names and read as of that app; a skill and an MCP server shared as kinds of their own,
//! a bundle naming no kind or several refused; the text by locale, a plain string in the maker's language, one locale
//! set, marked and cleared; and the CLI's words.

use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use mcp_host::catalogue::{self, Bundle, Catalogue, Share, app_key, source_hash};
use mcp_host::install;
use mcp_host::placed::Catalogue as Placed;
use mcp_host::registry::Registry;
use serde_json::{Value, json};

use crate::registry::AccountService;

const SCHEMA: &str = r#"{ "type": "object", "properties": { "text": { "type": "string" } } }"#;

// A personal app's bundle: `tool.toml` naming one entry `my/<entry>`, its schema, and its code, with what `more`
// adds to the manifest.
fn bundle(name: &str, entry: &str, more: &str) -> Bundle {
  let manifest = format!(
    "[tool]\nname = \"{name}\"\nprotocol = 1\nrun = [\"./run.sh\"]\n\n[[entry]]\nnamespace = \"my\"\nname = \"{entry}\"\ntitle = \"Weekly summary\"\ndescription = \"Sums up the week.\"\ninput = \"input.json\"\ncapabilities = [\"my.own\"]\n{more}"
  );

  Bundle {
    files: BTreeMap::from([
      ("tool.toml".to_owned(), manifest),
      ("input.json".to_owned(), SCHEMA.to_owned()),
      ("run.sh".to_owned(), "#!/bin/sh\necho '{}'\n".to_owned()),
    ]),
  }
}

fn share_of<'a>(version: &'a str, bundle: Bundle) -> Share<'a> {
  Share {
    maker: "17",
    unit: "40",
    version,
    targets: json!([{ "team": true }, { "heads": true }]),
    bundle,
    review_required: false,
    language: None,
  }
}

// A skill's bundle: `SKILL.md` with the front matter given, and a reference file.
fn skill(front: &str) -> Bundle {
  Bundle {
    files: BTreeMap::from([
      (
        "SKILL.md".to_owned(),
        format!("---\n{front}---\n\nWhen asked for the week, read the notes.\n"),
      ),
      ("reference/notes.md".to_owned(), "Where the notes stand.\n".to_owned()),
    ]),
  }
}

// An MCP server's bundle: `mcp.toml` holding the `[server]` table given and what `more` adds.
fn mcp(server: &str, more: &str) -> Bundle {
  Bundle {
    files: BTreeMap::from([(
      "mcp.toml".to_owned(),
      format!(
        "[server]\nname = \"tracker\"\ntitle = \"Tracker\"\ndescription = \"Reads the tracker.\"\n{server}\n{more}"
      ),
    )]),
  }
}

fn shared(kept: &Catalogue, service: &AccountService, asked: &Share) -> Result<catalogue::Record, String> {
  catalogue::share(kept, Some(&service.chain()), asked, &mut Vec::new())
}

#[test]
fn the_source_hash_is_one_for_the_same_files_and_another_for_any_change() {
  let files = bundle("weekly", "weekly_summary", "").files;
  let mut changed = files.clone();

  changed.insert("run.sh".to_owned(), "#!/bin/sh\necho '{ }'\n".to_owned());

  let reordered: BTreeMap<String, String> = files.clone().into_iter().rev().collect();

  assert_eq!(source_hash(&files), source_hash(&reordered));
  assert_ne!(source_hash(&files), source_hash(&changed));
  assert_eq!(source_hash(&files).len(), 64);
}

#[test]
fn a_shared_apps_key_names_its_company_maker_and_name_within_an_app_keys_length() {
  assert_eq!(app_key("3", "17", "weekly_summary"), "shared-3-17-weekly-summary");

  let long = app_key("3", "17", "a_name_far_longer_than_an_app_key_could_ever_hold_at_all");

  assert!(long.starts_with("shared-3-17-") && long.len() == "shared-3-17-".len() + 12, "{long}");
  assert!(bmo_tool::manifest::app_key(&long));
}

#[test]
fn a_share_declares_the_app_neither_offered_nor_on_and_keeps_its_version_and_source() {
  let dir = tempfile::tempdir().unwrap();
  let service = AccountService::start(false);
  let kept = Catalogue::of(dir.path(), "3").unwrap();
  let mut said = Vec::new();
  let record = catalogue::share(
    &kept,
    Some(&service.chain()),
    &share_of("v1", bundle("weekly", "weekly_summary", "")),
    &mut said,
  )
  .unwrap();

  assert_eq!(record.id, "17-weekly");
  assert_eq!(record.key, "shared-3-17-weekly");
  assert_eq!(record.title, "Weekly summary");
  assert_eq!(record.owner, "17");
  assert_eq!(record.versions[0].review.state, "pending");
  assert!(!record.versions[0].approval.required);

  let declared = service.kept();

  assert_eq!(declared[0].0, "/chain/kinds/app.shared-3-17-weekly.offered");
  assert_eq!(declared[0].2["default"], false);
  assert_eq!(declared[1].0, "/chain/kinds/app.shared-3-17-weekly.on");
  assert_eq!(declared[1].2["default"], false);
  assert_eq!(declared[1].2["personRaises"], true);

  let source = kept.source("17-weekly", None).unwrap();

  assert_eq!(source["sourceHash"], record.versions[0].source_hash);
  assert_eq!(source["files"]["run.sh"], "#!/bin/sh\necho '{}'\n");

  let again = catalogue::share(
    &kept,
    Some(&service.chain()),
    &share_of("v1", bundle("weekly", "weekly_summary", "")),
    &mut said,
  );

  assert!(again.unwrap_err().contains("shared version v1 already"));

  // A bundle changed on disk since it was shared is never handed out.
  let file = dir.path().join("tool-host/catalogue/3/sources/17-weekly/v1.json");
  let text = fs::read_to_string(&file).unwrap().replace("echo", "curl");

  fs::write(&file, text).unwrap();
  assert!(kept.source("17-weekly", None).unwrap_err().contains("does not match the hash"));
}

#[test]
fn a_share_is_refused_for_what_no_personal_app_may_be_and_without_an_account_service() {
  let dir = tempfile::tempdir().unwrap();
  let service = AccountService::start(false);
  let kept = Catalogue::of(dir.path(), "3").unwrap();
  let mut said = Vec::new();
  let mut outside = bundle("weekly", "weekly_summary", "");

  outside.files.insert("../escape".to_owned(), String::new());

  let refused = |asked: &Share, chain: bool| {
    catalogue::share(&kept, chain.then(|| service.chain()).as_ref(), asked, &mut Vec::new()).unwrap_err()
  };

  assert!(refused(&share_of("v1", outside), true).contains("outside the app's folder"));
  assert!(
    refused(
      &share_of("v1", bundle("weekly", "weekly_summary", "[permissions]\nnetwork = [\"*\"]\n")),
      true
    )
    .contains("network")
  );
  assert!(
    refused(&share_of("v1", bundle("weekly", "weekly_summary", "")), false).contains("no account service")
  );
  assert!(Catalogue::of(dir.path(), "../2").is_err());
  assert!(kept.list().is_empty());
  assert!(catalogue::share(&kept, None, &share_of("v1", Bundle::default()), &mut said).is_err());
}

#[test]
fn copies_reviews_approvals_withdrawals_owners_and_promotions_are_kept() {
  let dir = tempfile::tempdir().unwrap();
  let service = AccountService::start(false);
  let kept = Catalogue::of(dir.path(), "3").unwrap();
  let mut said = Vec::new();

  catalogue::share(
    &kept,
    Some(&service.chain()),
    &share_of("v1", bundle("weekly", "weekly_summary", "")),
    &mut said,
  )
  .unwrap();

  let record = catalogue::install(&kept, "17-weekly", "21", "v1", "self").unwrap();

  assert_eq!(record.installs["21"].how, "self");
  assert!(catalogue::install(&kept, "17-weekly", "21", "v9", "self").is_err());
  assert!(catalogue::install(&kept, "17-weekly", "21", "v1", "borrowed").is_err());

  let record =
    catalogue::review(&kept, "17-weekly", "v1", "written", "Sums up the week; reaches nothing else.")
      .unwrap();

  assert_eq!(record.versions[0].review.state, "written");
  assert!(catalogue::review(&kept, "17-weekly", "v1", "guessed", "").is_err());

  let record = catalogue::approve(&kept, "17-weekly", "v1", "9").unwrap();

  assert_eq!(record.versions[0].approval.by.as_deref(), Some("9"));

  let record = catalogue::withdraw(&kept, "17-weekly").unwrap();

  assert!(record.withdrawn.is_some());
  assert_eq!(catalogue::pass(&kept, "17", "9").unwrap(), vec!["17-weekly".to_owned()]);
  assert_eq!(kept.get("17-weekly").unwrap().owner, "9");
  assert_eq!(kept.get("17-weekly").unwrap().maker, "17");

  catalogue::promote(&kept, "17-weekly", "9", "Every team could use it").unwrap();

  let promotions = catalogue::promotions(dir.path());

  assert_eq!(promotions.len(), 1);
  assert_eq!(promotions[0]["company"], "3");
  assert_eq!(promotions[0]["note"], "Every team could use it");

  // A new version of a withdrawn app stands it again.
  let record = catalogue::share(
    &kept,
    Some(&service.chain()),
    &share_of("v2", bundle("weekly", "weekly_summary", "")),
    &mut said,
  )
  .unwrap();

  assert!(record.withdrawn.is_none());
  assert_eq!(record.versions.len(), 2);
  assert_eq!(catalogue::uninstall(&kept, "17-weekly", "21").unwrap().installs.len(), 0);
  assert!(kept.seen_at("21").is_none());
  assert!(kept.seen("21").unwrap() > 0);
  assert!(kept.seen_at("21").is_some());
}

// "Put a personal tool" (docs/registry.md): a received tool is put with the app the catalogue names, and the host
// reads it as of that app whatever its maker's manifest names.
#[test]
fn a_received_tool_is_read_as_of_the_shared_app_its_pointer_names() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let registry = Registry::of(&data);
  let folder = dir.path().join("weekly");
  let mut said = Vec::new();

  fs::create_dir_all(&folder).unwrap();

  for (path, text) in &bundle(
    "weekly",
    "weekly_summary",
    "\n[app]\nkey = \"makers-own\"\nname = \"Weekly\"\ndescription = \"Mine.\"\n",
  )
  .files
  {
    fs::write(folder.join(path), text).unwrap();
  }

  assert!(
    install::put_owned(&registry, "21", &folder, (Some("v1"), Some("Not A Key")), &mut said).is_err()
  );
  install::put_owned(&registry, "21", &folder, (Some("v1"), Some("shared-3-17-weekly")), &mut said)
    .unwrap();

  let pointer = registry.owned("21").get("weekly").unwrap();

  assert_eq!(pointer.app.as_deref(), Some("shared-3-17-weekly"));

  let (placed, faults) = Placed::read_all(None, Some(&registry), None);

  assert!(faults.is_empty(), "{faults:?}");

  let tool = &placed.personal["21"][0];

  assert_eq!(tool.manifest.app.as_ref().unwrap().key, "shared-3-17-weekly");
  assert_eq!(tool.manifest.app.as_ref().unwrap().name, "Weekly".into());
}

#[test]
fn the_cli_prints_what_the_catalogue_keeps_as_json() {
  let dir = tempfile::tempdir().unwrap();
  let pocket = |words: &[&str]| {
    Command::new(env!("CARGO_BIN_EXE_bmo-pocket"))
      .arg("--data-root")
      .arg(dir.path())
      .args(words)
      .env_remove("WEB_ADDRESS")
      .env_remove("WEB_TOKEN")
      .output()
      .unwrap()
  };
  let listed = pocket(&["catalogue", "list", "--company", "3", "--reader", "21"]);

  assert_eq!(listed.status.code(), Some(0));

  let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();

  assert_eq!(listed, json!({ "apps": [], "seenAt": null }));
  assert_eq!(pocket(&["catalogue", "show", "--company", "3", "17-weekly"]).status.code(), Some(1));
  assert_eq!(pocket(&["catalogue", "frobnicate"]).status.code(), Some(2));

  let promotions: Value = serde_json::from_slice(&pocket(&["catalogue", "promotions"]).stdout).unwrap();

  assert_eq!(promotions, json!([]));
}

#[test]
fn a_skill_is_shared_as_a_kind_of_its_own_asking_for_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let service = AccountService::start(false);
  let kept = Catalogue::of(dir.path(), "3").unwrap();
  let record = shared(
    &kept,
    &service,
    &share_of(
      "v1",
      skill(
        "name: week-notes\ndescription: Use when the person asks about their week.\nallowed-tools: Read\n",
      ),
    ),
  )
  .unwrap();

  assert_eq!(record.kind, "skill");
  assert_eq!(record.id, "17-week-notes");
  assert_eq!(record.key, "shared-3-17-week-notes");
  assert_eq!(record.title, "week-notes");
  assert_eq!(record.description, "Use when the person asks about their week.");
  assert_eq!(record.versions[0].permissions, catalogue::Permissions::default());
  assert_eq!(service.kept()[0].0, "/chain/kinds/app.shared-3-17-week-notes.offered");

  let refused = |bundle: Bundle| shared(&kept, &service, &share_of("v9", bundle)).unwrap_err();

  assert!(refused(skill("name: Week Notes\ndescription: x\n")).contains("no skill's name"));
  assert!(refused(skill("name: week-notes\ndescription: \"\"\n")).contains("no description"));
  assert!(refused(skill("name: [week\n")).contains("does not read"));

  let mut open = skill("");

  open.files.insert("SKILL.md".to_owned(), "No front matter here.\n".to_owned());
  assert!(refused(open).contains("no front matter"));
}

#[test]
fn an_mcp_server_asks_for_its_address_host_and_its_secret_and_holds_to_its_transport() {
  let dir = tempfile::tempdir().unwrap();
  let service = AccountService::start(false);
  let kept = Catalogue::of(dir.path(), "3").unwrap();
  let record = shared(
    &kept,
    &service,
    &share_of(
      "v1",
      mcp(
        "transport = \"remote\"\nurl = \"https://mcp.example.com:8443/mcp\"\nauth = \"bearer\"\nsecret = \"tracker-token\"\n",
        "[permissions]\nnetwork = [\"files.example.com\"]\n",
      ),
    ),
  )
  .unwrap();

  assert_eq!(record.kind, "mcp");
  assert_eq!(record.id, "17-tracker");
  assert_eq!(record.versions[0].permissions.network, ["files.example.com", "mcp.example.com:8443"]);
  assert_eq!(record.versions[0].permissions.secrets, ["tracker-token"]);

  let local = shared(
    &kept,
    &service,
    &share_of("v2", mcp("transport = \"local\"\ncommand = [\"python3\", \"server.py\"]\n", "")),
  )
  .unwrap();

  assert_eq!(local.versions[1].permissions, catalogue::Permissions::default());

  let refused = |server: &str| shared(&kept, &service, &share_of("v9", mcp(server, ""))).unwrap_err();

  assert!(refused("transport = \"local\"\n").contains("names its command"));
  assert!(
    refused("transport = \"remote\"\nurl = \"ftp://mcp.example.com\"\n").contains("no http or https")
  );
  assert!(
    refused("transport = \"remote\"\nurl = \"https://mcp.example.com\"\nauth = \"bearer\"\n")
      .contains("auth")
  );
  assert!(
    refused(
      "transport = \"remote\"\nurl = \"https://mcp.example.com\"\nauth = \"header\"\nsecret = \"t\"\n"
    )
    .contains("header names its header")
  );
  assert!(refused("transport = \"pigeon\"\n").contains("no transport"));
  assert!(
    refused("transport = \"local\"\ncommand = [\"x\"]\nvalue = \"sk-1\"\n").contains("does not read")
  );
}

#[test]
fn a_bundle_naming_no_kind_or_several_is_refused_and_an_item_keeps_its_kind() {
  let dir = tempfile::tempdir().unwrap();
  let service = AccountService::start(false);
  let kept = Catalogue::of(dir.path(), "3").unwrap();
  let mut both = bundle("weekly", "weekly_summary", "");

  both.files.extend(skill("name: weekly\ndescription: x\n").files);

  assert!(shared(&kept, &service, &share_of("v1", both)).unwrap_err().contains("more than one"));

  let none = Bundle {
    files: BTreeMap::from([("readme.md".to_owned(), "hi".to_owned())]),
  };

  assert!(shared(&kept, &service, &share_of("v1", none)).unwrap_err().contains("holds none"));

  shared(
    &kept,
    &service,
    &share_of("v1", mcp("transport = \"local\"\ncommand = [\"./s\"]\n", "")),
  )
  .unwrap();

  let tool = bundle("tracker", "tracker_read", "");

  assert!(
    shared(&kept, &service, &share_of("v2", tool))
      .unwrap_err()
      .contains("shared as mcp, never as app")
  );
}

#[test]
fn text_stands_by_locale_a_plain_string_in_the_makers_language_and_one_locale_is_set_and_cleared() {
  let dir = tempfile::tempdir().unwrap();
  let service = AccountService::start(false);
  let kept = Catalogue::of(dir.path(), "3").unwrap();
  let mut asked = share_of("v1", bundle("weekly", "weekly_summary", ""));

  asked.language = Some("ko");

  let record = shared(&kept, &service, &asked).unwrap();

  assert_eq!(record.language.as_deref(), Some("ko"));
  assert_eq!(record.titles, BTreeMap::from([("ko".to_owned(), "Weekly summary".to_owned())]));

  // An item naming several locales keeps each; none is required beside the one it holds.
  let several = bundle(
    "weekly",
    "weekly_summary",
    "\n[app]\nkey = \"w\"\nlanguage = \"en\"\nname = { en = \"Weekly\", ja = \"週報\" }\ndescription = { en = \"Sums up the week.\", ja = \"週をまとめる。\" }\n",
  );
  let record = shared(&kept, &service, &share_of("v2", several)).unwrap();

  assert_eq!(record.language.as_deref(), Some("en"));
  assert_eq!(record.title, "Weekly");
  assert_eq!(record.titles.len(), 3);
  assert_eq!(record.titles["ja"], "週報");

  let record =
    catalogue::set_text(&kept, "17-weekly", "zh-CN", Some(("周报", "总结一周。")), true).unwrap();

  assert!(record.machine.contains("zh-CN"));

  let record =
    catalogue::set_text(&kept, "17-weekly", "zh-CN", Some(("周报", "汇总本周。")), false).unwrap();

  assert!(!record.machine.contains("zh-CN"));
  assert_eq!(record.descriptions["zh-CN"], "汇总本周。");
  assert!(catalogue::set_text(&kept, "17-weekly", "fr", Some(("x", "y")), false).is_err());
  assert!(catalogue::set_text(&kept, "17-weekly", "ko", Some(("", "y")), false).is_err());

  let record = catalogue::set_text(&kept, "17-weekly", "zh-CN", None, false).unwrap();

  assert!(!record.titles.contains_key("zh-CN"));

  for locale in ["ko", "ja"] {
    catalogue::set_text(&kept, "17-weekly", locale, None, false).unwrap();
  }

  assert!(
    catalogue::set_text(&kept, "17-weekly", "en", None, false)
      .unwrap_err()
      .contains("stays")
  );

  // A record kept before kinds and locales stood reads as an app's with no text by locale.
  let file = dir.path().join("tool-host/catalogue/3/apps/17-weekly.json");
  let mut old: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();

  for field in ["kind", "language", "titles", "descriptions", "machine"] {
    old.as_object_mut().unwrap().remove(field);
  }

  fs::write(&file, old.to_string()).unwrap();

  let read = kept.get("17-weekly").unwrap();

  assert_eq!(read.kind, "app");
  assert!(read.titles.is_empty());
}

#[test]
fn the_cli_sets_a_locales_text() {
  let dir = tempfile::tempdir().unwrap();
  let service = AccountService::start(false);
  let kept = Catalogue::of(dir.path(), "3").unwrap();

  shared(&kept, &service, &share_of("v1", bundle("weekly", "weekly_summary", ""))).unwrap();

  let pocket = |words: &[&str]| {
    Command::new(env!("CARGO_BIN_EXE_bmo-pocket"))
      .arg("--data-root")
      .arg(dir.path())
      .args(words)
      .output()
      .unwrap()
  };
  let set = pocket(&[
    "catalogue",
    "text",
    "--company",
    "3",
    "17-weekly",
    "--locale",
    "ko",
    "--title",
    "Weekly in ko",
    "--description",
    "Sums up the week, in ko.",
    "--machine",
  ]);

  assert_eq!(set.status.code(), Some(0), "{}", String::from_utf8_lossy(&set.stderr));

  let record: Value = serde_json::from_slice(&set.stdout).unwrap();

  assert_eq!(record["titles"]["ko"], "Weekly in ko");
  assert_eq!(record["machine"], json!(["ko"]));
  assert_eq!(
    pocket(&["catalogue", "text", "--company", "3", "17-weekly", "--locale", "ko", "--clear"])
      .status
      .code(),
    Some(0)
  );
  assert_eq!(
    pocket(&["catalogue", "text", "--company", "3", "17-weekly", "--locale", "ko"])
      .status
      .code(),
    Some(2)
  );
}

#[test]
fn a_memory_pack_beside_the_kind_file_is_kept_on_the_version_and_a_pack_out_of_shape_is_refused() {
  let dir = tempfile::tempdir().unwrap();
  let service = AccountService::start(false);
  let kept = Catalogue::of(dir.path(), "3").unwrap();
  let with_pack = |pack: &str| {
    let mut one = skill("name: week-notes\ndescription: Reads the week's notes.\n");

    one.files.insert("memory.toml".to_owned(), pack.to_owned());
    one
  };
  let record = shared(
    &kept,
    &service,
    &share_of(
      "v1",
      with_pack(
        "[[memory]]\nkey = \"notes-first\"\ntype = \"tip\"\ntext = \"Read the notes before the calendar.\"\nrecall_condition = \"the person asks about their week\"\nimportance = 6\n",
      ),
    ),
  )
  .unwrap();

  assert_eq!(record.kind, "skill", "the pack names no kind of its own");
  assert_eq!(
    record.versions[0].memories,
    vec![catalogue::PackedMemory {
      key: "notes-first".to_owned(),
      kind: "tip".to_owned(),
      text: "Read the notes before the calendar.".to_owned(),
      recall_condition: "the person asks about their week".to_owned(),
      importance: Some(6),
    }]
  );

  let refused =
    |version: &str, pack: &str| shared(&kept, &service, &share_of(version, with_pack(pack))).unwrap_err();
  let row = |key: &str, kind: &str| {
    format!(
      "[[memory]]\nkey = \"{key}\"\ntype = \"{kind}\"\ntext = \"Do it.\"\nrecall_condition = \"when asked\"\n"
    )
  };

  assert!(refused("v2", &row("x", "fact")).contains("tip or check"));
  assert!(refused("v3", &row("Bad Key", "tip")).contains("no key"));
  assert!(refused("v4", &format!("{}{}", row("a", "tip"), row("a", "check"))).contains("twice"));
  assert!(
    refused(
      "v5",
      "[[memory]]\nkey = \"a\"\ntype = \"tip\"\ntext = \"x\"\nrecall_condition = \"y\"\nsubjects = []\n"
    )
    .contains("does not read")
  );
  assert!(refused("v6", "").contains("1 to 20"));
}

// An item's bundle with `bmo-extension.toml` at its root beside its kind file, holding `manifest`.
fn extended(mut one: Bundle, manifest: &str) -> Bundle {
  one.files.insert("bmo-extension.toml".to_owned(), manifest.to_owned());
  one
}

#[test]
fn an_extension_manifest_wraps_each_kind_and_its_texts_stand_over_the_kind_files() {
  let dir = tempfile::tempdir().unwrap();
  let service = AccountService::start(false);
  let kept = Catalogue::of(dir.path(), "3").unwrap();
  let mut app = extended(
    bundle("weekly", "summary", ""),
    "name = \"weekly\"\ndisplay_name = \"Weekly digest\"\ndescription = \"The week in one page.\"\npublisher = \"team-tools\"\nversion = \"1.2.0\"\nicon = \"icon.svg\"\ncategories = [\"app\"]\nrepository = \"https://example.com/weekly\"\nengines = { bmo = \">=1.0\" }\ndependencies = [\"team-tools.calendar\"]\n\n[contributes]\napps = [\"tool.toml\"]\n",
  );

  app
    .files
    .insert("icon.svg".to_owned(), "<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_owned());

  let record = shared(&kept, &service, &share_of("v1", app)).unwrap();
  let extension = record.versions[0].extension.clone().expect("the manifest is kept on the version");

  assert_eq!((record.kind.as_str(), record.title.as_str()), ("app", "Weekly digest"));
  assert_eq!(record.description, "The week in one page.");
  assert_eq!(extension.publisher.as_deref(), Some("team-tools"));
  assert_eq!(extension.icon_svg.as_deref(), Some("<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
  assert_eq!(extension.contributes.apps, vec!["tool.toml".to_owned()]);

  let mut skill_one = extended(
    skill("name: week-notes\ndescription: Reads the week's notes.\n"),
    "name = \"week-notes\"\n[contributes]\nskills = [\"SKILL.md\"]\nmemory_packs = [\"memory.toml\"]\n",
  );

  skill_one.files.insert(
    "memory.toml".to_owned(),
    "[[memory]]\nkey = \"a\"\ntype = \"tip\"\ntext = \"Do it.\"\nrecall_condition = \"when asked\"\n"
      .to_owned(),
  );

  let skill_record = shared(&kept, &service, &share_of("v1", skill_one)).unwrap();

  assert_eq!(skill_record.kind, "skill");
  assert_eq!(skill_record.title, "week-notes", "a manifest giving no text leaves the kind file's");
  assert_eq!(skill_record.versions[0].memories.len(), 1);

  let server = extended(
    mcp("transport = \"remote\"\nurl = \"https://tracker.example.com/mcp\"\nauth = \"none\"", ""),
    "name = \"tracker\"\n[contributes]\nmcp_servers = [\"mcp.toml\"]\n",
  );

  assert_eq!(shared(&kept, &service, &share_of("v1", server)).unwrap().kind, "mcp");
}

#[test]
fn an_extension_manifest_out_of_shape_or_contributing_what_the_bundle_does_not_carry_is_refused() {
  let dir = tempfile::tempdir().unwrap();
  let service = AccountService::start(false);
  let kept = Catalogue::of(dir.path(), "3").unwrap();
  let refused = |version: &str, manifest: &str| {
    shared(
      &kept,
      &service,
      &share_of(version, extended(skill("name: week-notes\ndescription: Notes.\n"), manifest)),
    )
    .unwrap_err()
  };
  let named = "name = \"week-notes\"\n";

  assert!(refused("v1", "name = [\n").contains("does not read"));
  assert!(
    refused("v2", &format!("{named}colour = \"red\"\n")).contains("does not read"),
    "an unknown key"
  );
  assert!(
    refused("v3", "name = \"other\"\n[contributes]\nskills = [\"SKILL.md\"]\n").contains("the item is")
  );
  assert!(
    refused("v4", &format!("{named}[contributes]\nskills = [\"SKILL.md\"]\npersonas = [\"p.md\"]\n"))
      .contains("nothing receives yet")
  );
  assert!(
    refused(
      "v5",
      &format!("{named}[contributes]\nskills = [\"SKILL.md\"]\nsystem_prompts = [\"p.md\"]\n")
    )
    .contains("nothing receives yet")
  );
  assert!(
    refused("v6", &format!("{named}[contributes]\nskills = [\"reference/notes.md\"]\n"))
      .contains("one item")
  );
  assert!(
    refused("v7", &format!("{named}[contributes]\napps = [\"tool.toml\"]\n")).contains("one item"),
    "another kind's point"
  );
  assert!(
    refused("v8", &format!("{named}[contributes]\nmemory_packs = [\"memory.toml\"]\n"))
      .contains("does not hold")
  );
  assert!(refused("v9", named).contains("does not contribute the item's own"));
  assert!(
    refused(
      "v10",
      &format!("{named}publisher = \"Big Co\"\n[contributes]\nskills = [\"SKILL.md\"]\n")
    )
    .contains("no publisher's name")
  );
  assert!(
    refused("v11", &format!("{named}icon = \"icon.png\"\n[contributes]\nskills = [\"SKILL.md\"]\n"))
      .contains("no SVG")
  );
  assert!(
    refused("v12", &format!("{named}icon = \"icon.svg\"\n[contributes]\nskills = [\"SKILL.md\"]\n"))
      .contains("does not hold")
  );
  assert!(
    refused(
      "v13",
      &format!("{named}categories = [\"widget\"]\n[contributes]\nskills = [\"SKILL.md\"]\n")
    )
    .contains("category")
  );
  assert!(
    refused(
      "v14",
      &format!("{named}repository = \"http://x\"\n[contributes]\nskills = [\"SKILL.md\"]\n")
    )
    .contains("https")
  );
  assert!(
    refused(
      "v15",
      &format!("{named}dependencies = [\"calendar\"]\n[contributes]\nskills = [\"SKILL.md\"]\n")
    )
    .contains("<publisher>.<name>")
  );
}
