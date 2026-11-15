//! The shared storage layer: one SQLite file holding every credential, the
//! Device cache and the protected-Device list. It also carries the JSON
//! output and exit-code contract the CLI and the Web Panel both obey.

pub mod contract;
pub mod guard;
pub mod store;

pub use contract::{ErrorBody, Exit};
pub use guard::{GuardDecision, GuardError};
pub use store::{Pair, Store, StoreError};
