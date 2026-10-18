//! Checks that span the repository: no Hangul in a `.rs` file, no dependency on another unit but the base
//! libraries, and the tool's manifest naming the entries the tool answers, each with its schema beside it.

use std::path::{Path, PathBuf};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// The only `bmo-*` packages a dependency may name: the base libraries.
const BASE_LIBRARIES: [&str; 3] = ["bmo-chat", "bmo-chat-client", "bmo-ui"];

/// The Hangul blocks of Unicode: Jamo, compatibility Jamo, Jamo extended A and B, syllables, and the halfwidth
/// forms.
const HANGUL: [(u32, u32); 6] = [
  (0x1100, 0x11FF),
  (0x3130, 0x318F),
  (0xA960, 0xA97F),
  (0xAC00, 0xD7AF),
  (0xD7B0, 0xD7FF),
  (0xFFA0, 0xFFDC),
];

fn is_hangul(letter: char) -> bool {
  HANGUL.iter().any(|&(first, last)| (first..=last).contains(&u32::from(letter)))
}

// Every `.rs` file under a folder, at any depth.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
  let mut found = Vec::new();

  for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
    let path = entry.path();

    if path.is_dir() {
      found.extend(rust_files(&path));
    } else if path.extension().is_some_and(|extension| extension == "rs") {
      found.push(path);
    }
  }

  found
}

#[test]
fn no_rust_file_holds_hangul() {
  let root = Path::new(ROOT);
  let held: Vec<String> = ["src", "tests"]
    .iter()
    .flat_map(|dir| rust_files(&root.join(dir)))
    .filter(|path| std::fs::read_to_string(path).unwrap().chars().any(is_hangul))
    .map(|path| path.display().to_string())
    .collect();

  assert!(held.is_empty(), "Rust files holding Hangul: {held:?}");
}

#[test]
fn every_dependency_is_from_a_registry_or_a_base_library() {
  let manifest: toml::Table =
    std::fs::read_to_string(Path::new(ROOT).join("Cargo.toml")).unwrap().parse().unwrap();
  let refused: Vec<String> = ["dependencies", "dev-dependencies"]
    .iter()
    .filter_map(|table| manifest.get(*table).and_then(toml::Value::as_table))
    .flat_map(|table| table.iter())
    .filter(|(name, spec)| {
      let foreign = spec.get("path").is_some() || spec.get("git").is_some() || name.starts_with("bmo-");

      foreign && !BASE_LIBRARIES.contains(&name.as_str())
    })
    .map(|(name, _)| name.clone())
    .collect();

  assert!(refused.is_empty(), "dependencies from another repository: {refused:?}");
}

#[test]
fn the_manifest_names_the_entries_the_tool_answers_each_with_its_schema_beside_it() {
  let root = Path::new(ROOT);
  let manifest: toml::Table = std::fs::read_to_string(root.join("tool.toml")).unwrap().parse().unwrap();
  let entries = manifest["entry"].as_array().unwrap();
  let named: Vec<String> = entries
    .iter()
    .map(|entry| format!("{}/{}", entry["namespace"].as_str().unwrap(), entry["name"].as_str().unwrap()))
    .collect();

  assert_eq!(manifest["tool"]["name"].as_str(), Some("word-chain"));
  assert_eq!(named, ["bmo/word_chain_start", "bmo/word_chain_stop"]);

  for entry in entries {
    let schema = root.join(entry["input"].as_str().unwrap());
    let read: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&schema).unwrap()).unwrap();

    assert_eq!(read["type"], "object", "{}", schema.display());
  }
}
