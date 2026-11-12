//! What a session's tools keep for it while its record stands (docs/mcp-host.md, 「The session record」): one
//! value of each type a family of tools asks for, made the first time it is asked for and ended when the
//! record goes — revoked, lapsed, or dropped as the host stops. A web reading's cookies and its browser are
//! kept this way; nothing kept is ever written down.

use std::any::{Any, TypeId};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// A value a family of tools keeps per record, and how it ends.
pub trait Kept: Send + Sync + 'static {
  /// Everything the value holds outside this process ended: a browser, a child process, a place.
  fn close(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
    Box::pin(async {})
  }
}

type Entry = (TypeId, Arc<dyn Any + Send + Sync>, Arc<dyn Kept>);

// What one record keeps, and whether it has been ended once.
#[derive(Default)]
struct Held {
  closed: bool,
  entries: Vec<Entry>,
}

/// The values kept for one record. Cloning it shares them, so every call under the record meets the same.
#[derive(Clone, Default)]
pub struct Keeping(Arc<Mutex<Held>>);

impl Keeping {
  fn held(&self) -> std::sync::MutexGuard<'_, Held> {
    self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
  }

  /// The value of type `T` this record keeps, made by `make` where it keeps none yet.
  pub fn get_or_keep<T: Kept>(&self, make: impl FnOnce() -> T) -> Arc<T> {
    let mut held = self.held();

    if let Some((_, value, _)) = held.entries.iter().find(|(kind, _, _)| *kind == TypeId::of::<T>()) {
      return Arc::clone(value).downcast::<T>().expect("a kept value is stored under its own type");
    }

    let value = Arc::new(make());

    held.entries.push((TypeId::of::<T>(), value.clone(), value.clone()));
    value
  }

  /// How many values are kept now.
  pub fn len(&self) -> usize {
    self.held().entries.len()
  }

  pub fn is_empty(&self) -> bool {
    self.len() == 0
  }

  /// Whether the record has gone: its values ended once. A value kept after that is never ended by the record, so
  /// whoever keeps one to learn of the record's end asks this beside it.
  pub fn is_closed(&self) -> bool {
    self.held().closed
  }

  /// Every value ended and let go, as the record goes.
  pub async fn close(&self) {
    let taken: Vec<Entry> = {
      let mut held = self.held();

      held.closed = true;
      std::mem::take(&mut held.entries)
    };

    for (_, _, kept) in taken {
      kept.close().await;
    }
  }
}

impl fmt::Debug for Keeping {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Keeping({} kept)", self.len())
  }
}

// What a record keeps is never part of what the record says, so two records are the same whatever each keeps.
impl PartialEq for Keeping {
  fn eq(&self, _: &Self) -> bool {
    true
  }
}
