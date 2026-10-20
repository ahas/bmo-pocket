use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bmo_tool::{Kept, Session};
use mcp_host::records::{Records, now_ms};
use serde_json::json;

// A value a record keeps — a browser, in the web tools — that notes being ended.
struct Browser(Arc<AtomicUsize>);

impl Kept for Browser {
  fn close(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
    Box::pin(async {
      self.0.fetch_add(1, Ordering::SeqCst);
    })
  }
}

fn session(expires_at: Option<u64>) -> Session {
  serde_json::from_value(json!({
    "holderName": "reader",
    "namespaces": ["web"],
    "capabilities": ["web.read"],
    "grants": [],
    "expiresAt": expires_at,
    "createdAt": now_ms(),
  }))
  .unwrap()
}

// A record past its `expiresAt` loses what it keeps at the next sweep, and every one goes when the host stops.
#[tokio::test]
async fn a_record_past_its_expiry_is_dropped_at_the_next_sweep_and_its_kept_values_end() {
  let place = tempfile::tempdir().unwrap();
  let records = Records::load(place.path()).unwrap();
  let ended = Arc::new(AtomicUsize::new(0));
  let resident = records.issue(session(None)).unwrap();
  let run = records.issue(session(Some(now_ms() + 3_600_000))).unwrap();
  let lapsing = records.issue(session(Some(now_ms() + 50))).unwrap();

  for token in [&resident, &run, &lapsing] {
    records.get(token).unwrap().1.kept.get_or_keep(|| Browser(ended.clone()));
  }

  assert!(records.prune().is_empty());
  std::thread::sleep(std::time::Duration::from_millis(60));

  let gone = records.prune();

  assert_eq!(gone.len(), 1);

  for (_, session) in gone {
    session.kept.close().await;
  }

  assert_eq!(ended.load(Ordering::SeqCst), 1);
  assert!(records.get(&lapsing).is_none());
  assert!(records.get(&resident).is_some() && records.get(&run).is_some());
  // What the others keep lives in the host's own process, so it goes as the host stops: a browser is
  // started to die with the process that started it.
}
