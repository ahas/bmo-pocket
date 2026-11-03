use std::sync::Arc;

use mcp_host::schedule::{self, Schedules};
use serde_json::json;

use crate::units::fixture::Fixture;

// A schedule is due at its pattern's first time after the beat that first sees it, and is called once at a beat past
// that time, under a record of its own: the holder `schedule`, the entry's own namespace and capabilities, and the
// time it was due.
#[tokio::test]
async fn a_schedule_due_is_called_once_under_a_record_of_its_own() {
  let fixture = Fixture::new();
  let schedules = Arc::new(Schedules::default());
  let entries = fixture.host.every();
  let started = schedule::now_ms();

  assert!(schedules.beat(&entries, started).is_empty());

  let read = entries.iter().find(|entry| entry.full_name() == "notes/read_note").unwrap();
  let next = schedules.of(read, started)[0].next_at.unwrap();

  assert!(next > started);
  assert!(schedules.beat(&entries, next - 1).is_empty());

  let mut due = schedules.beat(&entries, next);

  assert_eq!(due.len(), 1);
  assert!(schedules.beat(&entries, next).is_empty(), "a running schedule is called again");

  let record = schedule::record(&due[0].entry, due[0].at);

  assert_eq!(record.holder_name, "schedule");
  assert!(record.uses("notes") && !record.uses("garden"));
  assert!(record.grants.is_empty());

  schedule::call(fixture.host.clone(), schedules.clone(), due.remove(0)).await;

  let handed = fixture.handed();

  assert_eq!(handed.len(), 1);
  assert_eq!(handed[0]["entry"], "notes/read_note");
  assert_eq!(handed[0]["arguments"], json!({ "text": "morning" }));
  assert_eq!(handed[0]["holderName"], "schedule");
  assert_eq!(handed[0]["capabilities"], json!(["notes.read"]));
  assert_eq!(handed[0]["scheduledAt"], next.to_string());

  let standing = &schedules.of(read, next)[0];

  assert!(!standing.running && standing.last_error.is_none() && standing.last_at == Some(next));
}

// A scheduled call of an entry declaring a grant carries that grant, so it passes the check a call does.
#[test]
fn a_scheduled_record_holds_its_entrys_grant() {
  let fixture = Fixture::new();
  let session = fixture.session(&["notes.write"], &["notes.keep"]);
  let write = fixture.host.find(&session, "notes", "write_note").unwrap();
  let record = schedule::record(&write, 5);

  assert!(record.granted("notes.keep"));
  assert!(record.holds(&["notes.write"]));
  assert_eq!(record.variables["BMO_SCHEDULED_AT"], "5");
}

// "no schedule is called from then on" (docs/mcp-host.md, 「The host's stop」): the beats end as the stop is asked for,
// so the wait for them ends though their beat is far off, and nothing was called.
#[tokio::test]
async fn the_schedules_are_kept_no_longer_once_the_host_is_stopping() {
  let fixture = Fixture::new();

  fixture.host.drain.stop();
  schedule::keep(
    fixture.host.clone(),
    Arc::new(Schedules::default()),
    std::time::Duration::from_secs(3600),
  )
  .await;

  assert!(fixture.handed().is_empty());
}
