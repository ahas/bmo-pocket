//! Which entries a session's namespaces list by name: those its record's `shown` names, by full name, as the
//! record's issuer resolved them (docs/toolbook.md, 「Which tools are resident」). Every other entry the session
//! holds is booked, and stands in its toolbook.

use bmo_tool::Session;

/// Whether the session's record lists the entry of this full name by name.
pub fn shows(session: &Session, entry: &str) -> bool {
  session.shown.contains(entry)
}
