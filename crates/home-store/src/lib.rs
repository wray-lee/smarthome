//! The shared storage layer: one SQLite file holding every credential, the
//! Device cache, the protected-Device list and the Tuya Local Keys. It also
//! carries the JSON output and exit-code contract the CLI and the Web Panel
//! both obey.

pub mod contract;
pub mod guard;
pub mod store;

pub use contract::{ErrorBody, Exit};
pub use guard::{GuardDecision, GuardError};
pub use store::{LocalKeyChange, LocalKeyRecord, Pair, Store, StoreError};
