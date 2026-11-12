//! A call's caller going away (docs/mcp-host.md, 「The host's stop」): the record a call runs under ending, which
//! every call under it learns of on whatever road it came by.

use std::future::Future;
use std::pin::Pin;

use bmo_tool::{Kept, Session};
use tokio::sync::watch;

// Kept beside a record for as long as it stands, and told once the record goes.
#[derive(Default)]
struct Ending(watch::Sender<bool>);

impl Kept for Ending {
  fn close(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
    self.0.send_replace(true);
    Box::pin(async {})
  }
}

/// Ends once the record `session` stands for has gone — revoked, lapsed or ended with the host — and at once where
/// it already has.
pub async fn record_gone(session: &Session) {
  let ending = session.kept.get_or_keep(Ending::default);

  // A record already gone ended its values before this one was kept, so this one is never told.
  if session.kept.is_closed() {
    return;
  }

  let _ = ending.0.subscribe().wait_for(|gone| *gone).await;
}

/// Whether the record `session` stands for has gone.
pub fn is_gone(session: &Session) -> bool {
  session.kept.is_closed()
}
