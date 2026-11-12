//! Checks that span the repository: what its packages may depend on, and that no code or test file holds
//! Hangul. Each check is a function over what it reads, held both to the repository and to a made-up case of
//! what it refuses, so a check that has gone blind fails too.

use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// The only packages from another repository a package here may depend on: the base libraries.
const BASE_LIBRARIES: [&str; 4] = ["bmo-chat", "bmo-chat-client", "bmo-console-face", "bmo-ui"];

/// The dependency tables of a `package.json`.
const PACKAGE_TABLES: [&str; 4] =
  ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"];

/// The extensions of the code and test files that may hold no Hangul. Data a test reads, such as a fixture
/// file, has an extension of its own and is not held to this.
const CODE_EXTENSIONS: [&str; 3] = ["rs", "ts", "py"];

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

fn root() -> PathBuf {
  PathBuf::from(ROOT)
}

fn metadata() -> Value {
  let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
  let output = Command::new(cargo)
    .args(["metadata", "--no-deps", "--format-version", "1"])
    .current_dir(ROOT)
    .output()
    .expect("cargo metadata runs");

  assert!(
    output.status.success(),
    "cargo metadata failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );
  serde_json::from_slice(&output.stdout).expect("cargo metadata answers JSON")
}

/// Every file git tracks or would track: the committed ones and the new ones it does not ignore.
fn tracked_files() -> Vec<String> {
  let output = Command::new("git")
    .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
    .current_dir(ROOT)
    .output()
    .expect("git ls-files runs");

  assert!(
    output.status.success(),
    "git ls-files failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );

  let mut files: Vec<String> = String::from_utf8(output.stdout)
    .expect("tracked paths are UTF-8")
    .split('\0')
    .filter(|path| !path.is_empty() && Path::new(ROOT).join(path).is_file())
    .map(str::to_owned)
    .collect();

  files.sort();
  files.dedup();
  files
}

/// A path with its `.` and `..` parts taken away, read without the file system, since a path a manifest names
/// need not exist.
fn lexical(path: &Path) -> PathBuf {
  let mut clean = PathBuf::new();

  for part in path.components() {
    match part {
      Component::CurDir => {}
      Component::ParentDir => {
        clean.pop();
      }
      other => clean.push(other),
    }
  }

  clean
}

fn is_inside(root: &Path, path: &Path) -> bool {
  lexical(path).starts_with(lexical(root))
}

fn is_base_library(name: &str) -> bool {
  BASE_LIBRARIES.contains(&name)
}

/// Whether a package's name says it is one of this product's units.
fn is_unit_name(name: &str) -> bool {
  name.starts_with("bmo-")
    || name.starts_with("bmo_")
    || name.starts_with("@bmo/")
    || name.strip_prefix('@').is_some_and(|rest| rest.starts_with("bmo-"))
}

/// The crate dependencies `cargo metadata` lists that the repository may not hold, each as
/// `<manifest>: <crate> (<why>)`: a path outside the repository, a git source, or another unit's crate,
/// unless the crate is a base library.
fn refused_crates(metadata: &Value, root: &Path) -> Vec<String> {
  let mut refused = Vec::new();

  for package in metadata["packages"].as_array().into_iter().flatten() {
    let manifest = package["manifest_path"].as_str().unwrap_or("a manifest");

    for dependency in package["dependencies"].as_array().into_iter().flatten() {
      let name = dependency["name"].as_str().unwrap_or_default();

      if is_base_library(name) {
        continue;
      }

      let why = match (dependency["path"].as_str(), dependency["source"].as_str()) {
        (Some(path), _) if is_inside(root, Path::new(path)) => None,
        (Some(_), _) => Some("a path outside the repository"),
        (None, Some(source)) if source.starts_with("git+") => Some("a git source"),
        _ if is_unit_name(name) => Some("another unit's crate"),
        _ => None,
      };

      if let Some(why) = why {
        refused.push(format!("{manifest}: {name} ({why})"));
      }
    }
  }

  refused
}

/// The dependencies a `package.json` names that the repository may not hold, read as `refused_crates` reads a
/// crate's: a file or link path outside the repository, a git or web source, or another unit's package.
fn refused_packages(manifest: &Path, package: &Value, root: &Path) -> Vec<String> {
  let folder = manifest.parent().unwrap_or(root);
  let mut refused = Vec::new();

  for table in PACKAGE_TABLES {
    for (name, spec) in package[table].as_object().into_iter().flatten() {
      if is_base_library(name) {
        continue;
      }

      let spec = spec.as_str().unwrap_or_default();
      let local = ["file:", "link:", "portal:"].iter().find_map(|scheme| spec.strip_prefix(scheme));
      let why = match local {
        Some(path) if is_inside(root, &folder.join(path)) => None,
        Some(_) => Some("a path outside the repository"),
        None if spec.starts_with("workspace:") => None,
        None if spec.contains("://") || spec.starts_with("git") || spec.starts_with("github:") => {
          Some("a git or web source")
        }
        None if is_unit_name(name) => Some("another unit's package"),
        None => None,
      };

      if let Some(why) = why {
        refused.push(format!("{}: {name} ({why})", manifest.display()));
      }
    }
  }

  refused
}

/// The requirements a `pyproject.toml` lists that the repository may not hold: a direct reference (`name @
/// url`), which names no registry, or another unit's package.
fn refused_requirements(manifest: &Path, project: &toml::Table) -> Vec<String> {
  let requirements = project
    .get("project")
    .and_then(|table| table.get("dependencies"))
    .and_then(toml::Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(toml::Value::as_str);
  let mut refused = Vec::new();

  for requirement in requirements {
    let name: String = requirement
      .chars()
      .take_while(|letter| letter.is_alphanumeric() || "-_.".contains(*letter))
      .collect();

    if is_base_library(&name) {
      continue;
    }

    let why = if requirement.contains(" @ ") {
      Some("a direct reference")
    } else if is_unit_name(&name) {
      Some("another unit's package")
    } else {
      None
    };

    if let Some(why) = why {
      refused.push(format!("{}: {name} ({why})", manifest.display()));
    }
  }

  refused
}

fn is_hangul(letter: char) -> bool {
  let point = u32::from(letter);

  HANGUL.iter().any(|&(first, last)| (first..=last).contains(&point))
}

/// The numbers of the lines of a text that hold Hangul, counted from 1.
fn hangul_lines(text: &str) -> Vec<usize> {
  text
    .lines()
    .enumerate()
    .filter(|(_, line)| line.chars().any(is_hangul))
    .map(|(index, _)| index + 1)
    .collect()
}

fn is_code_file(path: &str) -> bool {
  Path::new(path)
    .extension()
    .and_then(|extension| extension.to_str())
    .is_some_and(|extension| CODE_EXTENSIONS.contains(&extension))
}

#[test]
fn every_dependency_is_from_a_registry_inside_the_repository_or_a_base_library() {
  let root = root();
  let mut refused = refused_crates(&metadata(), &root);

  for path in tracked_files() {
    let manifest = root.join(&path);

    match manifest.file_name().and_then(|name| name.to_str()) {
      Some("package.json") => {
        let package: Value = serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap())
          .unwrap_or_else(|why| panic!("{path} does not read as JSON: {why}"));

        refused.extend(refused_packages(&manifest, &package, &root));
      }
      Some("pyproject.toml") => {
        let project: toml::Table = std::fs::read_to_string(&manifest)
          .unwrap()
          .parse()
          .unwrap_or_else(|why| panic!("{path} does not read as TOML: {why}"));

        refused.extend(refused_requirements(&manifest, &project));
      }
      _ => {}
    }
  }

  assert!(refused.is_empty(), "dependencies the repository may not hold:\n{}", refused.join("\n"));
}

#[test]
fn the_dependency_check_refuses_another_units_package_a_path_outside_and_a_git_source() {
  let root = Path::new("/work/pocket");
  let metadata = json!({ "packages": [{
    "manifest_path": "/work/pocket/host/Cargo.toml",
    "dependencies": [
      { "name": "bmo-tool", "source": null, "path": "/work/pocket/sdk/contract" },
      { "name": "bmo-chat", "source": "git+https://example.com/bmo-chat", "path": null },
      { "name": "serde", "source": "registry+https://github.com/rust-lang/crates.io-index", "path": null },
      { "name": "bmo-policy", "source": null, "path": "/work/pocket/../old/bmo-policy" },
      { "name": "helper", "source": "git+https://example.com/helper", "path": null },
      { "name": "bmo-agency", "source": "registry+https://github.com/rust-lang/crates.io-index", "path": null },
    ],
  }]});

  assert_eq!(
    refused_crates(&metadata, root),
    [
      "/work/pocket/host/Cargo.toml: bmo-policy (a path outside the repository)",
      "/work/pocket/host/Cargo.toml: helper (a git source)",
      "/work/pocket/host/Cargo.toml: bmo-agency (another unit's crate)",
    ]
  );

  let manifest = root.join("sdk/kit/package.json");
  let package = json!({
    "dependencies": { "bmo-ui": "file:../../../ui", "left-pad": "1.3.0", "@bmo/testkit": "workspace:*" },
    "devDependencies": { "@bmo/bot": "1.0.0", "shared": "file:../../../shared", "near": "file:../contract" },
    "optionalDependencies": { "remote": "github:example/remote" },
  });

  assert_eq!(
    refused_packages(&manifest, &package, root),
    [
      "/work/pocket/sdk/kit/package.json: @bmo/bot (another unit's package)",
      "/work/pocket/sdk/kit/package.json: shared (a path outside the repository)",
      "/work/pocket/sdk/kit/package.json: remote (a git or web source)",
    ]
  );

  let manifest = root.join("sdk/kit/pyproject.toml");
  let project: toml::Table =
    "[project]\ndependencies = [\"requests>=2\", \"bmo-brain==1.0\", \"helper @ https://example.com/helper.whl\"]"
      .parse()
      .unwrap();

  assert_eq!(
    refused_requirements(&manifest, &project),
    [
      "/work/pocket/sdk/kit/pyproject.toml: bmo-brain (another unit's package)",
      "/work/pocket/sdk/kit/pyproject.toml: helper (a direct reference)",
    ]
  );
}

#[test]
fn no_code_or_test_file_holds_hangul() {
  let root = root();
  let held: Vec<String> = tracked_files()
    .into_iter()
    .filter(|path| is_code_file(path))
    .filter_map(|path| {
      let text = std::fs::read_to_string(root.join(&path)).ok()?;
      let lines = hangul_lines(&text);

      (!lines.is_empty()).then(|| format!("{path}: lines {lines:?}"))
    })
    .collect();

  assert!(held.is_empty(), "code and test files holding Hangul:\n{}", held.join("\n"));
}

#[test]
fn the_hangul_check_finds_a_syllable_and_a_letter_and_passes_other_scripts() {
  let syllable = char::from_u32(0xAC00).unwrap();
  let letter = char::from_u32(0x3131).unwrap();
  let text = format!("plain\nsays {syllable}\ncaf\u{e9} and \u{4e2d}\n{letter}\n");

  assert_eq!(hangul_lines(&text), [2, 4]);
  assert!(
    is_code_file("host/src/lock.rs") && is_code_file("sdk/kit/tool.ts") && is_code_file("kit/tool.py")
  );
  assert!(!is_code_file("tests/fixtures/words.xml") && !is_code_file("docs/api.md"));
}
