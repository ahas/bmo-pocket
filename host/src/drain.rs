//! The host's stop (docs/mcp-host.md, 「The host's stop」): whether it has been asked for, and what the host still
//! holds that it waits on.

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

/// Whether the host was asked to stop, the calls it is asking now, and the work beyond those it finishes first.
#[derive(Default)]
pub struct Drain {
  stopping: watch::Sender<bool>,
  calls: Count,
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

  /// One call of a tool in hand, which `calls_in_hand` counts.
  pub fn call(&self) -> Held {
    self.calls.hold()
  }

  /// Work the host keeps on with around a call — an asynchronous call's record, a scheduled call's ending — which
  /// the stop waits on and `calls_in_hand` does not count.
  pub fn finishing(&self) -> Held {
    self.finishing.hold()
  }

  /// How many calls of tools are in hand now.
  pub fn calls_in_hand(&self) -> usize {
    *self.calls.0.borrow()
  }

  /// Ends once nothing is held. Work kept on with holds the calls it makes, so it is waited on first.
  pub async fn emptied(&self) {
    self.finishing.emptied().await;
    self.calls.emptied().await;
  }
}
