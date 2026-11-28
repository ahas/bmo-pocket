//! An item's extension manifest, `bmo-extension.toml` at its bundle's root (docs/catalogue.md, 「The extension
//! manifest」): the item described as an extension, and what it contributes checked against the one item the bundle
//! carries.

use std::collections::BTreeMap;

use bmo_tool::manifest::{MANIFEST, Text};
use serde::{Deserialize, Serialize};

use crate::catalogue::{Bundle, KIND_FILES, MCP, MEMORY_PACK, SKILL};

/// The manifest's file at a bundle's root.
pub const EXTENSION: &str = "bmo-extension.toml";

// The bounds a manifest keeps.
const PUBLISHER_MOST: usize = 64;
const VERSION_MOST: usize = 64;
const CATEGORIES: [&str; 7] = ["persona", "memory-pack", "system-prompt", "tool", "app", "skill", "mcp"];

/// What an extension contributes, by contribution point: each a list of the bundle's paths.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contributes {
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub tools: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub apps: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub skills: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub mcp_servers: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub memory_packs: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub personas: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub system_prompts: Vec<String>,
}

/// The manifest as a share reads it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
  name: String,
  #[serde(default)]
  display_name: Option<Text>,
  #[serde(default)]
  description: Option<Text>,
  #[serde(default)]
  publisher: Option<String>,
  #[serde(default)]
  version: Option<String>,
  #[serde(default)]
  icon: Option<String>,
  #[serde(default)]
  categories: Vec<String>,
  #[serde(default)]
  repository: Option<String>,
  #[serde(default)]
  engines: BTreeMap<String, String>,
  #[serde(default)]
  dependencies: Vec<String>,
  #[serde(default)]
  contributes: Contributes,
}

/// The manifest as a version keeps it (docs/catalogue.md, 「A version」).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub display_name: Option<Text>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<Text>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub publisher: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub version: Option<String>,
  /// The path of the icon the bundle holds, and its SVG text.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub icon: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub icon_svg: Option<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub categories: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub repository: Option<String>,
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub engines: BTreeMap<String, String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub dependencies: Vec<String>,
  pub contributes: Contributes,
}

fn publisher_name(name: &str) -> bool {
  name.len() <= PUBLISHER_MOST
    && name.starts_with(|first: char| first.is_ascii_lowercase())
    && name.chars().all(|one| one.is_ascii_lowercase() || one.is_ascii_digit() || one == '-')
}

// `<publisher>.<name>`, each part a publisher's name's alphabet.
fn dependency_name(name: &str) -> bool {
  name
    .split_once('.')
    .is_some_and(|(publisher, item)| publisher_name(publisher) && publisher_name(item))
}

/// The bundle's extension manifest read and checked against the item of `kind` named `name` it carries, none where it
/// carries none (docs/catalogue.md, 「The extension manifest」).
pub fn read(bundle: &Bundle, kind: &str, name: &str) -> Result<Option<Extension>, String> {
  let Some(text) = bundle.files.get(EXTENSION) else {
    return Ok(None);
  };
  let file: File = toml::from_str(text).map_err(|why| format!("its {EXTENSION} does not read: {why}"))?;

  if file.name != name {
    return Err(format!("its {EXTENSION} names the item {:?}, and the item is {name:?}", file.name));
  }

  if let Some(publisher) = file.publisher.as_deref().filter(|publisher| !publisher_name(publisher)) {
    return Err(format!(
      "its {EXTENSION} names the publisher {publisher:?}, which is no publisher's name"
    ));
  }

  if let Some(version) = file
    .version
    .as_deref()
    .filter(|version| version.is_empty() || version.len() > VERSION_MOST)
  {
    return Err(format!(
      "its {EXTENSION} names the version {version:?}; it is 1 to {VERSION_MOST} characters"
    ));
  }

  if let Some(category) = file.categories.iter().find(|category| !CATEGORIES.contains(&category.as_str())) {
    return Err(format!(
      "its {EXTENSION} names the category {category:?}; it is one of {}",
      CATEGORIES.join(", ")
    ));
  }

  if let Some(repository) =
    file.repository.as_deref().filter(|repository| !repository.starts_with("https://"))
  {
    return Err(format!(
      "its {EXTENSION} names the repository {repository:?}, which is no https address"
    ));
  }

  if let Some(dependency) = file.dependencies.iter().find(|dependency| !dependency_name(dependency)) {
    return Err(format!("its {EXTENSION} names the dependency {dependency:?}; it is <publisher>.<name>"));
  }

  let icon_svg =
    match file.icon.as_deref() {
      None => None,
      Some(icon) if !icon.ends_with(".svg") => {
        return Err(format!("its {EXTENSION} names the icon {icon:?}, which is no SVG file"));
      }
      Some(icon) => Some(bundle.files.get(icon).cloned().ok_or_else(|| {
        format!("its {EXTENSION} names the icon {icon:?}, which the bundle does not hold")
      })?),
    };

  contributions_faults(bundle, kind, &file.contributes)?;

  Ok(Some(Extension {
    name: file.name,
    display_name: file.display_name,
    description: file.description,
    publisher: file.publisher,
    version: file.version,
    icon: file.icon,
    icon_svg,
    categories: file.categories,
    repository: file.repository,
    engines: file.engines,
    dependencies: file.dependencies,
    contributes: file.contributes,
  }))
}

// What the manifest contributes held to the one item the bundle carries: the kind file under the point its kind is
// put through, the memory pack under its own, and nothing else; the points with no receiver yet refused as such.
fn contributions_faults(bundle: &Bundle, kind: &str, contributes: &Contributes) -> Result<(), String> {
  for (point, named) in
    [("personas", &contributes.personas), ("system_prompts", &contributes.system_prompts)]
  {
    if !named.is_empty() {
      return Err(format!("its {EXTENSION} contributes {point}, which nothing receives yet"));
    }
  }

  let kind_file = KIND_FILES.iter().find(|(named, _)| *named == kind).map_or(MANIFEST, |(_, file)| *file);
  let at_points: [(&str, &Vec<String>, &[&str]); 5] = [
    ("tools", &contributes.tools, &[MANIFEST]),
    ("apps", &contributes.apps, &[MANIFEST]),
    ("skills", &contributes.skills, &[SKILL]),
    ("mcp_servers", &contributes.mcp_servers, &[MCP]),
    ("memory_packs", &contributes.memory_packs, &[MEMORY_PACK]),
  ];
  let mut named_kind_file = false;

  for (point, named, takes) in at_points {
    for path in named {
      if !takes.contains(&path.as_str()) || (path != MEMORY_PACK && path != kind_file) {
        return Err(format!(
          "its {EXTENSION} contributes {path:?} as {point}; a bundle carries one item, so its {point} name only the \
           file at its root a {kind} puts through it"
        ));
      }

      if !bundle.files.contains_key(path) {
        return Err(format!("its {EXTENSION} contributes {path:?}, which the bundle does not hold"));
      }

      named_kind_file |= path == kind_file;
    }
  }

  if !named_kind_file {
    return Err(format!("its {EXTENSION} does not contribute the item's own {kind_file}"));
  }

  if bundle.files.contains_key(MEMORY_PACK)
    && !contributes.memory_packs.iter().any(|path| path == MEMORY_PACK)
  {
    return Err(format!(
      "its {EXTENSION} does not contribute the bundle's {MEMORY_PACK} as memory_packs"
    ));
  }

  Ok(())
}
