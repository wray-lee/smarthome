//! The one write guard. Every path that changes a Device — either Backend,
//! CLI or Web Panel — asks this first, and a protected Device stops here
//! before any upstream call. Nothing overrides it.

use crate::store::{Pair, Store, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardDecision {
    Allowed,
    Protected,
}

#[derive(Debug)]
pub enum GuardError {
    Store(StoreError),
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GuardError {}

impl From<StoreError> for GuardError {
    fn from(e: StoreError) -> Self {
        GuardError::Store(e)
    }
}

/// May this Device be written? Reads never ask; writes always do.
pub fn check(store: &Store, backend: &str, device_id: &str) -> Result<GuardDecision, GuardError> {
    Ok(if store.is_protected(backend, device_id)? {
        GuardDecision::Protected
    } else {
        GuardDecision::Allowed
    })
}

/// [`check`], as the error a write path returns: exit 6, `protected`.
pub fn require(
    store: &Store,
    backend: &'static str,
    device_id: &str,
) -> Result<(), (crate::contract::ErrorBody, crate::contract::Exit)> {
    let decision = check(store, backend, device_id).map_err(|e| {
        (
            crate::contract::ErrorBody::new("storage", e.to_string()),
            crate::contract::Exit::Upstream,
        )
    })?;
    match decision {
        GuardDecision::Allowed => Ok(()),
        GuardDecision::Protected => Err((
            crate::contract::ErrorBody::new(
                "protected",
                format!(
                    "{backend} device {device_id} is protected; remove it with \
                     `home protect --remove` first"
                ),
            )
            .backend(backend),
            crate::contract::Exit::Refused,
        )),
    }
}

/// The protected list as the JSON `home protect --list` prints.
pub fn list(store: &Store) -> Result<Vec<Pair>, GuardError> {
    Ok(store.protected_devices()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_protected_device_is_refused_and_an_unknown_one_is_allowed() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            check(&store, "google_home", "dev-1").unwrap(),
            GuardDecision::Allowed
        );
        store.protect("google_home", "dev-1").unwrap();
        assert_eq!(
            check(&store, "google_home", "dev-1").unwrap(),
            GuardDecision::Protected
        );
        // The same id on the other Backend is a different Device.
        assert_eq!(
            check(&store, "tuya", "dev-1").unwrap(),
            GuardDecision::Allowed
        );
    }

    #[test]
    fn require_carries_exit_six_and_the_protected_code() {
        let store = Store::open_in_memory().unwrap();
        store.protect("google_home", "dev-1").unwrap();
        let (body, exit) = require(&store, "google_home", "dev-1").unwrap_err();
        assert_eq!(exit.code(), 6);
        assert_eq!(body.code, "protected");
        assert!(body.message.contains("dev-1"));
        assert!(require(&store, "google_home", "dev-2").is_ok());
    }
}
