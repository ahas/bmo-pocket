use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bmo_tool::{Keeping, Kept};

#[derive(Default)]
struct Counter {
  calls: AtomicUsize,
  closed: Arc<AtomicUsize>,
}

impl Kept for Counter {
  fn close(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
    Box::pin(async {
      self.closed.fetch_add(1, Ordering::SeqCst);
    })
  }
}

// One record's state is kept across its calls and ended, browser and all, when the record is revoked.
#[tokio::test]
async fn a_record_keeps_one_value_of_a_type_across_calls_and_ends_it_when_it_goes() {
  let keeping = Keeping::default();
  let closed = Arc::new(AtomicUsize::new(0));
  let made = |closed: &Arc<AtomicUsize>| Counter {
    calls: AtomicUsize::new(0),
    closed: closed.clone(),
  };

  keeping.get_or_keep(|| made(&closed)).calls.fetch_add(1, Ordering::SeqCst);
  keeping.clone().get_or_keep(|| made(&closed)).calls.fetch_add(1, Ordering::SeqCst);
  assert_eq!(keeping.get_or_keep(|| made(&closed)).calls.load(Ordering::SeqCst), 2);
  assert_eq!(keeping.len(), 1);
  assert!(!keeping.is_closed());

  keeping.close().await;
  assert_eq!(closed.load(Ordering::SeqCst), 1);
  assert!(keeping.is_empty());
  assert!(keeping.clone().is_closed(), "every clone of the record's keeping says it has gone");

  // A call made under the record again starts afresh.
  assert_eq!(keeping.get_or_keep(|| made(&closed)).calls.load(Ordering::SeqCst), 0);
}
