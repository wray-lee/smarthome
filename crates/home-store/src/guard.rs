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
    if store.is_protected(backend, device_id)? {
        Ok(GuardDecision::Protected)
    } else {
        Ok(GuardDecision::Allowed)
    }
}

/// [`check`], as the error a write path returns: exit 6, `protected`.
pub fn require(
    store: &Store,
    backend: &'static str,
    device_id: &str,
) -> Result<(), (crate::contract::ErrorBody, crate::contract::Exit)> {
    require_with(store, backend, device_id, false)
}

/// [`check`], with a one-time override. `force` lets exactly this call
/// through a protected Device; it never edits the protected list.
pub fn check_with(
    store: &Store,
    backend: &str,
    device_id: &str,
    force: bool,
) -> Result<GuardDecision, GuardError> {
    if force {
        return Ok(GuardDecision::Allowed);
    }
    check(store, backend, device_id)
}

/// [`require`], with a one-time override; see [`check_with`].
pub fn require_with(
    store: &Store,
    backend: &'static str,
    device_id: &str,
    force: bool,
) -> Result<(), (crate::contract::ErrorBody, crate::contract::Exit)> {
    let decision = check_with(store, backend, device_id, force).map_err(|e| {
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

    #[test]
    fn force_allows_one_write_and_never_touches_the_protected_list() {
        let store = Store::open_in_memory().unwrap();
        store.protect("google_home", "dev-1").unwrap();
        assert_eq!(
            check_with(&store, "google_home", "dev-1", true).unwrap(),
            GuardDecision::Allowed
        );
        assert!(require_with(&store, "google_home", "dev-1", true).is_ok());
        // The list is exactly what it was: protection is not consumed.
        assert_eq!(list(&store).unwrap(), store.protected_devices().unwrap());
        assert_eq!(list(&store).unwrap().len(), 1);
        // Without force the same Device is still refused.
        assert_eq!(
            check(&store, "google_home", "dev-1").unwrap(),
            GuardDecision::Protected
        );
        assert_eq!(
            require_with(&store, "google_home", "dev-1", false)
                .unwrap_err()
                .0
                .code,
            "protected"
        );
    }

    #[test]
    fn the_force_parameter_defaults_are_the_old_behaviour() {
        let store = Store::open_in_memory().unwrap();
        store.protect("tuya", "dev-1").unwrap();
        assert_eq!(
            check_with(&store, "tuya", "dev-1", false).unwrap(),
            check(&store, "tuya", "dev-1").unwrap()
        );
        assert_eq!(
            require_with(&store, "tuya", "dev-1", false)
                .unwrap_err()
                .1
                .code(),
            require(&store, "tuya", "dev-1").unwrap_err().1.code()
        );
        assert_eq!(
            check_with(&store, "tuya", "dev-2", true).unwrap(),
            GuardDecision::Allowed
        );
    }
}
