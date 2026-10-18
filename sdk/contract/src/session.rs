//! The session record a call is made under, issued by the launcher with the token (docs/api.md, 「The
//! session record」).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One session's record. Its field names are camel case on the wire. A field the record does not know is
/// refused rather than dropped, so a launcher that hands something new is told rather than ignored.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Session {
  /// The run line the session belongs to, a name a tool may read.
  pub line: String,
  /// The role of the agent the session is issued for, where it has one, a name a tool may read.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub role: Option<String>,
  /// What the session may be shown, by the names the manifests declare, as the issuer resolved them.
  pub capabilities: BTreeSet<String>,
  /// The full names of the entries its namespaces list by name, as the issuer resolved them; every other entry
  /// the session holds stands in its toolbook.
  #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
  pub shown: BTreeSet<String>,
  /// The checkout the run works in, whose graph is read on every call.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub checkout: Option<PathBuf>,
  /// The data root the run reads.
  pub data_root: PathBuf,
  /// The archive address its tools read, where the issuer names one.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub archive: Option<PathBuf>,
  /// A resident program's place, where each call reads which run is going.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub place: Option<PathBuf>,
  /// The run's own variables, for a run spawned for itself: its name, the job runner's marker, the
  /// work-order step and every `BMO_TURN_` variable.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub run_variables: BTreeMap<String, String>,
  /// The fragment desk an editor's batch opened.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub fragment_desk: Option<PathBuf>,
  /// What a search, a browse or a branch is held to.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub research: Option<Research>,
  /// When a run's record lapses, in epoch milliseconds; a resident program's has none.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub deadline: Option<u64>,
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

  /// Every variable of the turn a call is made in, read afresh as [`Session::turn_variable`] reads one:
  /// the run's own variables, overlaid by those of the run its resident program's place names.
  pub fn turn_environment(&self) -> BTreeMap<String, String> {
    let mut turn = self.run_variables.clone();
    let from_place = self.place.as_ref().and_then(|place| {
      let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(place.join("run.json")).ok()?).ok()?;

      if run.get("person").and_then(serde_json::Value::as_bool) == Some(true) {
        return None;
      }

      run.get("env")?.as_object().cloned()
    });

    for (name, value) in from_place.into_iter().flatten() {
      let ours = ["BMO_RUN_NAME", "BMO_TASK_LOCKED", "BMO_WORK_ORDER_STEP"].contains(&name.as_str())
        || name.starts_with("BMO_TURN_");

      if let (true, Some(value)) = (ours, value.as_str()) {
        turn.insert(name, value.to_owned());
      }
    }

    turn
  }

  /// One variable of the turn a call is made in, read afresh: the run its resident program's place names —
  /// none for a person's turn — or the run's own variables for a run spawned for itself (docs/api.md,
  /// 「The session record」).
  pub fn turn_variable(&self, name: &str) -> Option<String> {
    let from_place = self.place.as_ref().and_then(|place| {
      let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(place.join("run.json")).ok()?).ok()?;

      if run.get("person").and_then(serde_json::Value::as_bool) == Some(true) {
        return None;
      }

      run.get("env")?.get(name)?.as_str().map(str::to_owned)
    });

    from_place.or_else(|| self.run_variables.get(name).cloned())
  }
}

/// What a research run's web tools are held to: when it closes, whose it is, how it fetches and how
/// many branches it may open.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Research {
  /// When research closes, in epoch milliseconds.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub closes_at: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub owner: Option<String>,
  /// `search` or `browse`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub source: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub role: Option<String>,
  /// How many branches the run may open.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub branches: Option<u32>,
  /// The room the research answers.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub chat: Option<i64>,
  /// How pages are fetched: directly, through a browser first, or directly only.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub strategy: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub host_interval_ms: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub engine_interval_ms: Option<u64>,
  /// The rulebooks `read_rulebook` hands out.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub rulebooks: Vec<String>,
}
