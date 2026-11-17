//! The session record a call is made under, issued with its token on the control socket (docs/api.md, 「The
//! session record」).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One grant a session holds: a resource it may act on, an opaque name its issuer and its tools agree on.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Granted {
  pub resource: String,
}

/// One session's record, as the host keeps it and hands it to every call. Its field names are camel case on the
/// wire. A field the record does not know is refused rather than dropped, so an issuer that hands something new is
/// told rather than ignored.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Session {
  /// Who holds the session, a name a tool may read and the host never interprets.
  pub holder_name: String,
  /// The namespaces the session may use.
  pub namespaces: BTreeSet<String>,
  /// What the session may be shown, by the names the manifests declare, as the issuer resolved them.
  pub capabilities: BTreeSet<String>,
  /// The resources the session may act on.
  pub grants: Vec<Granted>,
  /// The full names of the entries its namespaces list by name, as the issuer resolved them; every other entry
  /// the session holds stands in its toolbook.
  #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
  pub shown: BTreeSet<String>,
  /// The full names of the only entries the session reaches, those for the system alone among them; every entry
  /// its namespaces and capabilities give it where none. The host alone checks it, so a tool is handed the record
  /// without it.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub entries: Option<BTreeSet<String>>,
  /// Variables handed to every call's environment.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub variables: BTreeMap<String, String>,
  /// A file holding a JSON object of strings, read afresh at every call and laid over `variables`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub variables_file: Option<PathBuf>,
  /// The issuer's own metadata, handed to every call and never read by the host.
  #[serde(default, skip_serializing_if = "Map::is_empty")]
  pub labels: Map<String, Value>,
  /// When the session lapses, in epoch milliseconds; none stands until it is revoked.
  #[serde(default)]
  pub expires_at: Option<u64>,
  /// When it was issued, in epoch milliseconds.
  #[serde(default)]
  pub created_at: u64,
  /// What the session's tools keep for it while its record stands, ended when the record goes; never written
  /// down.
  #[serde(skip)]
  pub kept: crate::kept::Keeping,
}

impl Session {
  /// Whether the session holds every capability of these names.
  pub fn holds<S: AsRef<str>>(&self, wanted: &[S]) -> bool {
    wanted.iter().all(|one| self.capabilities.contains(one.as_ref()))
  }

  /// Whether the session may use this namespace.
  pub fn uses(&self, namespace: &str) -> bool {
    self.namespaces.contains(namespace)
  }

  /// Whether the session's grants hold this resource, compared whole.
  pub fn granted(&self, resource: &str) -> bool {
    self.grants.iter().any(|grant| grant.resource == resource)
  }

  /// Whether the session's `entries` lets it reach the entry of this full name: any, where it names none.
  pub fn reaches(&self, full: &str) -> bool {
    self.entries.as_ref().is_none_or(|entries| entries.contains(full))
  }

  /// Whether the session has lapsed at `now`, in epoch milliseconds.
  pub fn lapsed(&self, now: u64) -> bool {
    self.expires_at.is_some_and(|expires| expires <= now)
  }

  /// The variables its `variablesFile` holds, read afresh: none where it names no file, and why where the file
  /// does not stand or does not read as an object of strings.
  pub fn file_variables(&self) -> Result<BTreeMap<String, String>, String> {
    let Some(file) = &self.variables_file else {
      return Ok(BTreeMap::new());
    };
    let text = std::fs::read_to_string(file)
      .map_err(|why| format!("its variables file {}: {why}", file.display()))?;

    serde_json::from_str(&text)
      .map_err(|why| format!("its variables file {} is no object of strings: {why}", file.display()))
  }
}

/// What an issuer asks a session to be issued with: the record less the times the host answers, and how long
/// it stands.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionRequest {
  pub holder_name: String,
  pub namespaces: BTreeSet<String>,
  pub capabilities: BTreeSet<String>,
  pub grants: Vec<Granted>,
  #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
  pub shown: BTreeSet<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub entries: Option<BTreeSet<String>>,
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub variables: BTreeMap<String, String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub variables_file: Option<PathBuf>,
  #[serde(default, skip_serializing_if = "Map::is_empty")]
  pub labels: Map<String, Value>,
  /// How long the session stands, in milliseconds; none stands until it is revoked.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub ttl_ms: Option<u64>,
}

impl SessionRequest {
  /// The record it asks for, issued at `now`, in epoch milliseconds.
  pub fn issued_at(self, now: u64) -> Session {
    Session {
      holder_name: self.holder_name,
      namespaces: self.namespaces,
      capabilities: self.capabilities,
      grants: self.grants,
      shown: self.shown,
      entries: self.entries,
      variables: self.variables,
      variables_file: self.variables_file,
      labels: self.labels,
      expires_at: self.ttl_ms.map(|ttl| now.saturating_add(ttl)),
      created_at: now,
      kept: Default::default(),
    }
  }
}
