use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bmo_tool::manifest::MANIFEST;
use mcp_host::placed::{Catalogue, PLACED, Placed, concerns, moves};
use notify::EventKind;
use notify::event::{AccessKind, CreateKind, DataChange, MetadataKind, ModifyKind, RemoveKind, RenameMode};

#[test]
fn a_file_opened_or_read_moves_nothing_and_one_made_written_renamed_or_removed_does() {
  assert!(!moves(&EventKind::Access(AccessKind::Any)));
  assert!(!moves(&EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))));
  assert!(moves(&EventKind::Create(CreateKind::Folder)));
  assert!(moves(&EventKind::Modify(ModifyKind::Data(DataChange::Any))));
  assert!(moves(&EventKind::Modify(ModifyKind::Name(RenameMode::Both))));
  assert!(moves(&EventKind::Remove(RemoveKind::File)));
}

fn place(tools: &Path, folder: &str, manifest: &str) {
  let placed = tools.join(folder).join(PLACED);

  std::fs::create_dir_all(&placed).unwrap();
  std::fs::write(placed.join(MANIFEST), manifest).unwrap();
  std::fs::write(placed.join("in.json"), r#"{ "type": "object" }"#).unwrap();
}

fn manifest(name: &str, entry: &str) -> String {
  format!(
    "[tool]\nname = \"{name}\"\nprotocol = 1\nrun = [\"run\"]\ntimeout = 5\n\n[[entry]]\nnamespace = \"garden\"\nname = \"{entry}\"\ndescription = \"It does.\"\ninput = \"in.json\"\ncapabilities = [\"graph.read\"]\n"
  )
}

#[test]
fn a_change_beside_the_link_is_no_reason_to_read_and_one_to_the_link_or_under_tools_is() {
  let folder = PathBuf::from("/srv/tools");
  let links = [folder.join("deploy")];
  let folders = [folder.clone()];
  let concerned = |path: &str| concerns(&[PathBuf::from(path)], &links, &folders);

  assert!(concerned("/srv/tools/deploy"));
  assert!(concerned("/srv/tools/deploy-releases/0123456789ab/tools/echo/.placed"));
  assert!(!concerned("/srv/tools/deploy.incoming"));
  assert!(!concerned("/srv/tools/cargo-target"));
  assert!(concerns(&[], &links, &folders));
}

// The tools served, by folder.
fn served(placed: &Placed) -> Vec<String> {
  placed.current().tools.iter().map(|tool| tool.folder.clone()).collect()
}

// Waits up to two seconds for the catalogue to serve exactly `folders`.
async fn serves(placed: &Placed, folders: &[&str]) -> Vec<String> {
  for _ in 0..40 {
    if served(placed) == folders {
      break;
    }

    tokio::time::sleep(Duration::from_millis(50)).await;
  }

  served(placed)
}

#[tokio::test]
async fn a_flip_of_the_link_onto_another_release_is_followed_and_the_new_release_s_tools_read() {
  let root = tempfile::tempdir().unwrap();
  let (one, two) = (root.path().join("releases/one"), root.path().join("releases/two"));
  let link = root.path().join("deploy");

  place(&one.join("tools"), "alpha", &manifest("alpha", "alpha_one"));
  place(&two.join("tools"), "beta", &manifest("beta", "beta_one"));
  std::os::unix::fs::symlink(&one, &link).unwrap();

  let placed = Arc::new(Placed::new(Some(link.join("tools")), root.path().join("sockets")));

  placed.watch(Duration::from_millis(50)).unwrap();
  assert_eq!(served(&placed), ["alpha"]);

  // The deployment's flip: a new link renamed over the old one.
  std::os::unix::fs::symlink(&two, root.path().join("deploy.next")).unwrap();
  std::fs::rename(root.path().join("deploy.next"), &link).unwrap();
  assert_eq!(serves(&placed, &["beta"]).await, ["beta"]);

  // The new release's tools/ is the one watched now.
  place(&two.join("tools"), "gamma", &manifest("gamma", "gamma_one"));
  assert_eq!(serves(&placed, &["beta", "gamma"]).await, ["beta", "gamma"]);

  // And the old one's is not.
  place(&one.join("tools"), "delta", &manifest("delta", "delta_one"));
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert_eq!(served(&placed), ["beta", "gamma"]);
}

#[test]
fn each_placed_folder_is_read_and_one_that_does_not_read_or_takes_a_placed_name_is_left_out_with_why() {
  let tools = tempfile::tempdir().unwrap();
  let dir = tools.path();

  place(dir, "alpha", &manifest("alpha", "alpha_one"));
  place(dir, "beta", &manifest("beta", "alpha_one"));
  place(dir, "gamma", &manifest("other", "gamma_one"));
  place(dir, "_sdk", &manifest("_sdk", "nothing"));
  // A shared helper is found through the registry alone: one under the tools root is left out, and a folder of
  // helpers there is no tool.
  place(
    dir,
    "web",
    "[tool]\nname = \"web\"\nprotocol = 1\nrun = [\"web\"]\ntimeout = 5\n\n[helper]\ndescription = \"A web session.\"\n",
  );
  place(&dir.join("_shared"), "counter", &manifest("counter", "counter_one"));
  std::fs::create_dir_all(dir.join("unplaced")).unwrap();

  let (catalogue, faults) = Catalogue::read(dir);
  let folders: Vec<&str> = catalogue.tools.iter().map(|tool| tool.folder.as_str()).collect();

  assert_eq!(folders, ["alpha"]);
  assert!(catalogue.helpers.is_empty());
  assert_eq!(catalogue.namespaces(), BTreeSet::from(["garden".to_owned()]));
  assert_eq!(faults.len(), 3, "{faults:?}");
  assert!(
    faults
      .iter()
      .any(|fault| fault == "tools/web: a shared helper is found through the registry alone")
  );
  assert!(
    faults
      .iter()
      .any(|fault| fault == "tools/beta: garden/alpha_one is another tool's already")
  );
  assert!(faults.iter().any(|fault| fault.starts_with("tools/gamma: its name `other`")));
}
