//! The schedules (docs/mcp-host.md, 「Schedules」): each entry's `[[entry.schedule]]` read from the catalogue as it
//! stands at every beat, each called by the host itself at the times its pattern names, under a record made for
//! that call. Where each stands is kept in the host's process alone, and `GET /schedules` answers it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bmo_tool::{Granted, Session};
use chrono::{Local, TimeZone};
use serde_json::{Map, Value};

use crate::handler::{Entry, Host};

/// The holder a scheduled call's record names.
pub const HOLDER: &str = "schedule";

/// A schedule's place: its entry's full name, and its position among that entry's schedules.
pub type Key = (String, usize);

/// Where one schedule an entry declares stands, as `GET /schedules` answers it.
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduleState {
  /// The cron pattern its manifest declares.
  pub cron: String,
  /// The arguments each scheduled call is handed.
  pub arguments: Map<String, Value>,
  /// When it is next due, in epoch milliseconds; none where the pattern names no time to come.
  pub next_at: Option<i64>,
  /// When its last call was made since the host started, in epoch milliseconds.
  pub last_at: Option<i64>,
  /// Why its last call failed, where it did.
  pub last_error: Option<String>,
  /// Whether a call of it is running now.
  pub running: bool,
}

// Where one schedule stands.
struct Standing {
  cron: String,
  arguments: Map<String, Value>,
  next_at: Option<i64>,
  last_at: Option<i64>,
  last_error: Option<String>,
  running: bool,
}

impl Standing {
  fn state(&self) -> ScheduleState {
    ScheduleState {
      cron: self.cron.clone(),
      arguments: self.arguments.clone(),
      next_at: self.next_at,
      last_at: self.last_at,
      last_error: self.last_error.clone(),
      running: self.running,
    }
  }
}

/// A schedule a beat found due: its entry, the arguments it is handed and the time it was due.
pub struct Due {
  pub key: Key,
  pub entry: Entry,
  pub arguments: Map<String, Value>,
  pub at: i64,
}

/// Every schedule of the catalogue, and where each stands.
#[derive(Default)]
pub struct Schedules {
  standing: Mutex<BTreeMap<Key, Standing>>,
}

// The pattern's first time after `now`, in epoch milliseconds, read in the machine's local zone; none where it names
// no time to come.
fn next_after(pattern: &croner::Cron, now: i64) -> Option<i64> {
  let now = Local.timestamp_millis_opt(now).single()?;

  pattern.find_next_occurrence(&now, false).ok().map(|next| next.timestamp_millis())
}

// The text of a failed tool result, as its last error is kept.
fn failure_text(result: &Value) -> String {
  let said: Vec<&str> = result["content"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|item| item["text"].as_str())
    .collect();

  if said.is_empty() {
    "the tool answered a failed result".to_owned()
  } else {
    said.join("\n")
  }
}

/// The record a scheduled call of an entry is made under, due at `at`: the holder `schedule`, the entry's own
/// namespace, capabilities and grant, and the time it was due as `BMO_SCHEDULED_AT`. The host's variables
/// `--pass-env` names reach it as they reach every call.
pub fn record(entry: &Entry, at: i64) -> Session {
  Session {
    holder_name: HOLDER.to_owned(),
    namespaces: BTreeSet::from([entry.namespace().to_owned()]),
    capabilities: entry.capabilities().into_iter().collect(),
    grants: entry.grant().into_iter().map(|resource| Granted { resource }).collect(),
    variables: BTreeMap::from([("BMO_SCHEDULED_AT".to_owned(), at.to_string())]),
    created_at: u64::try_from(at).unwrap_or_default(),
    ..Session::default()
  }
}

impl Schedules {
  fn standing(&self) -> MutexGuard<'_, BTreeMap<Key, Standing>> {
    self.standing.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
  }

  /// One beat at `now`, in epoch milliseconds, over the catalogue's entries: a schedule newly seen, or whose pattern
  /// or arguments changed, is due at its pattern's first time after `now`; one whose entry is gone is dropped; and
  /// each due at or before `now` is due next at its first time after `now` and handed back, unless its last call
  /// still runs.
  pub fn beat(&self, entries: &[Entry], now: i64) -> Vec<Due> {
    let mut standing = self.standing();
    let mut seen = BTreeSet::new();
    let mut due = Vec::new();

    for entry in entries {
      for (at, schedule) in entry.declared().schedule.iter().enumerate() {
        // A manifest whose pattern does not read is left out of the catalogue, so every pattern here reads.
        let Ok(pattern) = schedule.pattern() else {
          continue;
        };
        let key = (entry.full_name(), at);

        seen.insert(key.clone());

        let same = standing
          .get(&key)
          .is_some_and(|was| was.cron == schedule.cron && was.arguments == schedule.arguments);

        if !same {
          standing.insert(
            key,
            Standing {
              cron: schedule.cron.clone(),
              arguments: schedule.arguments.clone(),
              next_at: next_after(&pattern, now),
              last_at: None,
              last_error: None,
              running: false,
            },
          );
          continue;
        }

        let Some(one) = standing.get_mut(&key) else {
          continue;
        };
        let Some(was_due) = one.next_at.filter(|next| *next <= now) else {
          continue;
        };

        one.next_at = next_after(&pattern, now);

        if one.running {
          continue;
        }

        one.running = true;
        one.last_at = Some(now);
        due.push(Due {
          key,
          entry: entry.clone(),
          arguments: one.arguments.clone(),
          at: was_due,
        });
      }
    }

    standing.retain(|key, _| seen.contains(key));
    due
  }

  /// A scheduled call ended: why it failed, where it did.
  pub fn finished(&self, key: &Key, error: Option<String>) {
    if let Some(one) = self.standing().get_mut(key) {
      one.running = false;
      one.last_error = error;
    }
  }

  /// Where each schedule of an entry stands, in the order its manifest declares them, as of `now`: one no beat has
  /// seen yet is next due at its pattern's first time after `now`.
  pub fn of(&self, entry: &Entry, now: i64) -> Vec<ScheduleState> {
    let standing = self.standing();
    let full = entry.full_name();

    entry
      .declared()
      .schedule
      .iter()
      .enumerate()
      .map(|(at, schedule)| match standing.get(&(full.clone(), at)) {
        Some(one) if one.cron == schedule.cron && one.arguments == schedule.arguments => one.state(),
        _ => ScheduleState {
          cron: schedule.cron.clone(),
          arguments: schedule.arguments.clone(),
          next_at: schedule.pattern().ok().and_then(|pattern| next_after(&pattern, now)),
          last_at: None,
          last_error: None,
          running: false,
        },
      })
      .collect()
  }
}

/// One scheduled call made: dispatched as a call through the control socket is, under its own record, which ends as
/// it answers; a failure, the host's refusal or the tool's own failed result, kept and said in the log.
pub async fn call(host: Arc<Host>, schedules: Arc<Schedules>, due: Due) {
  let session = record(&due.entry, due.at);
  let error = match host.attempt(&due.entry, &session, Value::Object(due.arguments)).await {
    Ok(result) if result["isError"] == true => Some(failure_text(&result)),
    Ok(_) => None,
    Err(refusal) => Some(refusal.said().to_owned()),
  };

  session.kept.close().await;

  if let Some(why) = &error {
    eprintln!("mcp-host: the scheduled call of {} failed: {why}", due.key.0);
  }

  schedules.finished(&due.key, error);
}

/// The schedules kept until the host is asked to stop: a beat every `every`, each schedule it finds due called on
/// a task of its own, which the stop waits on.
pub async fn keep(host: Arc<Host>, schedules: Arc<Schedules>, every: Duration) {
  while !host.drain.is_stopping() {
    for due in schedules.beat(&host.every(), now_ms()) {
      let finishing = host.drain.finishing();
      let called = call(host.clone(), schedules.clone(), due);

      tokio::spawn(async move {
        called.await;
        drop(finishing);
      });
    }

    tokio::time::sleep(every).await;
  }
}

/// Now, in epoch milliseconds, as the schedules read it.
pub fn now_ms() -> i64 {
  Local::now().timestamp_millis()
}
