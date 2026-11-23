//! The contract every tool of the tool host is written to (docs/api.md, 「The call protocol」 and 「The
//! manifest」).
//!
//! A tool declares what it is — its name, the namespace it is shown under, its description, its
//! input, its hints and the capabilities a session must hold to be shown it — and is placed as a
//! folder whose manifest the host reads at run time (`manifest`) and whose executable it asks over the
//! protocol standing here (`protocol`). Namespaces and capabilities are names the manifests declare and
//! a record's issuer grants; no list of them, and no list of tools, is kept in code.

mod answer;
mod ceiling;
mod input;
mod kept;
pub mod manifest;
pub mod protocol;
mod resources;
mod session;
mod tool;

pub use answer::{Answer, CallError, Content};
pub use ceiling::{Capped, MAX_ANSWER_CHARS, MAX_VALUE_CHARS, Said, cap_items, cap_value, count_text};
pub use input::{AtLeastOne, Digits, IsoDay, NonEmpty, Positive, WebAddress};
pub use kept::{Keeping, Kept};
pub use resources::Resources;
pub use session::{Granted, Session, SessionRequest, Withheld};
pub use tool::{Bundle, Grant, Held, Hints, Pending, Served, Tool, narrowed};
