//! The host's stop (docs/mcp-host.md, 「The host's stop」): whether it has been asked for, and what the host still
//! holds that it waits on.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;

/// One thing the stop waits on, let go as it is dropped.
pub struct Held(watch::Sender<usize>);

impl Drop for Held {
  fn drop(&mut self) {
    self.0.send_modify(|count| *count -= 1);
  }
}

#[derive(Default)]
struct Count(watch::Sender<usize>);

impl Count {
  fn hold(&self) -> Held {
    self.0.send_modify(|count| *count += 1);
    Held(self.0.clone())
  }

  async fn emptied(&self) {
    // The sender stands in `self`, so the wait ends only at the count it asks for.
    let _ = self.0.subscribe().wait_for(|count| *count == 0).await;
  }
}

/// One call in hand as the host names it: what it asks, whose record it runs under, and when it started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InHand {
  /// The entry's full name, or the tool's own name for a resource listing or read.
  pub entry: String,
  /// The holder of the record the call runs under.
  pub holder: String,
  /// When it started, in epoch milliseconds.
  pub started_at: u64,
}

// The calls in hand by the order they were taken in, each under a number of its own.
type Book = Arc<Mutex<(u64, BTreeMap<u64, InHand>)>>;

/// One call in hand, counted and named until it is dropped.
pub struct Call {
  _held: Held,
  number: u64,
  book: Book,
}

impl Drop for Call {
  fn drop(&mut self) {
    self.book.lock().unwrap_or_else(PoisonError::into_inner).1.remove(&self.number);
  }
}

/// Whether the host was asked to stop, the calls it is asking now, and the work beyond those it finishes first.
#[derive(Default)]
pub struct Drain {
  stopping: watch::Sender<bool>,
  calls: Count,
  book: Book,
  finishing: Count,
}

impl Drain {
  /// The stop asked for: from here no new call is taken.
  pub fn stop(&self) {
    self.stopping.send_replace(true);
  }

  /// Whether the stop has been asked for.
  pub fn is_stopping(&self) -> bool {
    *self.stopping.borrow()
  }

  /// Ends once the stop has been asked for, at once where it already has.
  pub async fn stopping(&self) {
    let _ = self.stopping.subscribe().wait_for(|stopping| *stopping).await;
  }

  /// One call of a tool in hand, which `in_hand` names.
  pub fn call(&self, named: InHand) -> Call {
    let number = {
      let mut book = self.book.lock().unwrap_or_else(PoisonError::into_inner);

      book.0 += 1;

      let number = book.0;

      book.1.insert(number, named);
      number
    };

    Call {
      _held: self.calls.hold(),
      number,
      book: self.book.clone(),
    }
  }

  /// Work the host keeps on with around a call — an asynchronous call's record, a scheduled call's ending — which
  /// the stop waits on and `in_hand` does not name.
  pub fn finishing(&self) -> Held {
    self.finishing.hold()
  }

  /// Each call of a tool in hand now, in the order they were taken in.
  pub fn in_hand(&self) -> Vec<InHand> {
    self.book.lock().unwrap_or_else(PoisonError::into_inner).1.values().cloned().collect()
  }

  /// Ends once nothing is held. Work kept on with holds the calls it makes, so it is waited on first.
  pub async fn emptied(&self) {
    self.finishing.emptied().await;
    self.calls.emptied().await;
  }
}
