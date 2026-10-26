//! The host's event stream (docs/api.md, 「Events」): the events of this run, numbered from above the moment it
//! started so a later run's numbers are greater than any earlier run's, the last `WINDOW` of them held in memory,
//! and every reader woken as one is published.

use std::collections::VecDeque;
use std::sync::Mutex;

use bmo_pocket_client::{Event, EventKind, ToolChange};
use tokio::sync::watch;

use crate::records::now_ms;

/// How many events the host holds for a reader resuming.
pub const WINDOW: usize = 500;

/// The events of the run, and the number of the last one published, which every reader watches.
pub struct Events {
  held: Mutex<Held>,
  latest: watch::Sender<i64>,
  /// Whether the host stops, which ends every stream so the host's serving lets go.
  stopping: watch::Sender<bool>,
}

struct Held {
  events: VecDeque<Event>,
  /// The number the oldest event held follows: a cursor below it names an event no longer held.
  floor: i64,
  /// The number of the last event published, or the floor where none was.
  last: i64,
}

/// The cursor names an event older than the window held, or one of an earlier run.
#[derive(Debug, PartialEq, Eq)]
pub struct Gone {
  /// The number the oldest event held follows.
  pub floor: i64,
}

impl Default for Events {
  fn default() -> Self {
    Self::starting_at(now_ms())
  }
}

impl Events {
  /// The events of a run that started at this moment, in epoch milliseconds: numbered from above it times 1000.
  pub fn starting_at(started_ms: u64) -> Self {
    let floor = i64::try_from(started_ms).unwrap_or(i64::MAX / 1000).saturating_mul(1000);
    let (latest, _) = watch::channel(floor);
    let (stopping, _) = watch::channel(false);

    Self {
      held: Mutex::new(Held {
        events: VecDeque::new(),
        floor,
        last: floor,
      }),
      latest,
      stopping,
    }
  }

  fn held(&self) -> std::sync::MutexGuard<'_, Held> {
    self.held.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
  }

  /// One event published under the next number, the oldest let go past the window, and every reader woken.
  pub fn publish(&self, kind: EventKind, data: ToolChange) -> Event {
    let event = {
      let mut held = self.held();

      held.last += 1;

      let event = Event {
        id: held.last,
        kind,
        at: now_ms(),
        data,
      };

      held.events.push_back(event.clone());

      while held.events.len() > WINDOW {
        if let Some(gone) = held.events.pop_front() {
          held.floor = gone.id;
        }
      }

      event
    };

    self.latest.send_replace(event.id);
    event
  }

  /// The events held after the cursor, in order of number, or from the oldest held where none is given; refused
  /// where the cursor is below the window.
  pub fn after(&self, after: Option<i64>) -> Result<Vec<Event>, Gone> {
    let held = self.held();
    let after = after.unwrap_or(held.floor);

    if after < held.floor {
      return Err(Gone { floor: held.floor });
    }

    Ok(held.events.iter().filter(|event| event.id > after).cloned().collect())
  }

  /// The number of the last event published, where a reader with no cursor begins.
  pub fn last(&self) -> i64 {
    self.held().last
  }

  /// A watch on the last number published, which changes as each event is.
  pub fn watch(&self) -> watch::Receiver<i64> {
    self.latest.subscribe()
  }

  /// A watch on whether the host stops.
  pub fn stopping(&self) -> watch::Receiver<bool> {
    self.stopping.subscribe()
  }

  /// Every stream ended, as the host stops.
  pub fn stop(&self) {
    self.stopping.send_replace(true);
  }
}
