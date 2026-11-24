//! The platform administrator's deletion of an account or a company for good (docs/api.md, 「The console face」):
//! the count changes nothing and names what the removal then takes; the account's registry of personal tools goes
//! whole, its copy of a company's app and its seen mark go while the app stays, another account's stay, a company's
//! catalogue goes only for the company itself, and a second removal takes nothing.

use std::collections::BTreeMap;
use std::fs;

use mcp_host::catalogue::{self, Bundle, Catalogue, Share};
use mcp_host::registry::Registry;
use mcp_host::removal;
use serde_json::json;

use crate::registry::AccountService;

fn bundle() -> Bundle {
  let manifest = "[tool]\nname = \"weekly\"\nprotocol = 1\nrun = [\"./run.sh\"]\n\n[[entry]]\nnamespace = \"my\"\nname = \"weekly_summary\"\ntitle = \"Weekly summary\"\ndescription = \"Sums up the week.\"\ninput = \"input.json\"\ncapabilities = [\"my.own\"]\n";
  Bundle {
    files: BTreeMap::from([
      ("tool.toml".to_owned(), manifest.to_owned()),
      ("input.json".to_owned(), r#"{ "type": "object" }"#.to_owned()),
      ("run.sh".to_owned(), "#!/bin/sh\necho '{}'\n".to_owned()),
    ]),
  }
}

#[test]
fn an_accounts_tools_copies_and_marks_go_and_nothing_else_does() {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path();
  let service = AccountService::start(false);
  let kept = Catalogue::of(root, "3").unwrap();
  let share = Share {
    maker: "17",
    unit: "40",
    version: "v1",
    targets: json!([{ "team": true }]),
    bundle: bundle(),
    review_required: false,
    language: None,
  };
  catalogue::share(&kept, Some(&service.chain()), &share, &mut Vec::new()).unwrap();
  catalogue::install(&kept, "17-weekly", "21", "v1", "self").unwrap();
  catalogue::install(&kept, "17-weekly", "22", "v1", "self").unwrap();
  kept.seen("21").unwrap();
  let owners = Registry::of(root).owners_root();
  fs::create_dir_all(owners.join("21").join("registry")).unwrap();
  fs::create_dir_all(owners.join("22").join("registry")).unwrap();

  let counted = removal::account_held(root, "21");
  assert_eq!(counted, json!({ "personal-tools": 0, "installs": 1, "seen-marks": 1 }));
  assert!(kept.seen_at("21").is_some(), "a count changes nothing");

  let removed = removal::remove_account(root, "21").unwrap();
  assert_eq!(removed, counted);
  assert!(!owners.join("21").exists(), "the account's registry is taken whole");
  assert!(owners.join("22").exists(), "another account's stays");
  let record = kept.get("17-weekly").expect("the app stays in its company's catalogue");
  assert!(!record.installs.contains_key("21"));
  assert!(record.installs.contains_key("22"));
  assert!(kept.seen_at("21").is_none());

  let again = removal::remove_account(root, "21").unwrap();
  assert_eq!(again, json!({ "personal-tools": 0, "installs": 0, "seen-marks": 0 }));

  assert_eq!(removal::unit_held(root, "40", Some("3")).unwrap(), json!({ "catalogue-apps": 0 }));
  assert_eq!(removal::remove_unit(root, "3", Some("3")).unwrap(), json!({ "catalogue-apps": 1 }));
  assert!(
    !catalogue::catalogues(root).join("3").exists(),
    "the company's catalogue is taken whole"
  );
  assert_eq!(removal::remove_unit(root, "3", Some("3")).unwrap(), json!({ "catalogue-apps": 0 }));
  assert_eq!(removal::id("0"), None);
  assert_eq!(removal::id("seven"), None);
}
