//! `home tuya devices`: every Device of every home, and its Local Key saved.
//!
//! Ported from Alegzandr/lueur `crates/lueur-shell/src/tuya_cloud.rs`,
//! commit be7ba9b14be8fc1031ee962c9069c744e88e8252, MIT; Copyright (c) 2026
//! Lueur contributors. See `LICENSE-tuya` in this crate.
//!
//! The two calls are `GET /v1.0/m/life/users/homes` and, per home,
//! `GET /v1.0/m/life/ha/home/devices?homeId=<ownerId>` (the home's `ownerId`
//! is the id the device list wants, not its `id`). The device payload carries
//! `local_key`; it is saved through [`home_store::Store::save_local_key`] and
//! never leaves the Backend except through [`local_keys`].

use home_store::Store;
use serde_json::Value;

use crate::session::{Failure, Method, TuyaClient};

/// One Device as `home tuya devices` reports it. No `local_key`: stdout is
/// metadata only, and the keys come out of `home tuya keys` alone.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub online: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub backend: &'static str,
}

/// What happened to the Local Keys during a device list refresh, per Device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyChange {
    /// First time this Device's key was saved.
    First,
    /// The same key as before; only the observation time moved.
    Unchanged,
    /// The key differs from the stored one; the old key and the change time
    /// are kept in the history table.
    Changed,
}

/// The device list plus what the key save did.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub devices: Vec<Device>,
    pub key_changes: Vec<(String, KeyChange)>,
}

impl Listing {
    pub fn devices_json(&self) -> Value {
        serde_json::to_value(&self.devices).unwrap_or_else(|_| Value::Array(Vec::new()))
    }
}

/// Fetch every Device of every home, saving each Local Key on the way.
///
/// Reads never touch the guard: this Backend has no write command in this
/// ticket, and the guard is for writes (ticket 05).
pub fn list_devices(client: &TuyaClient<'_>) -> Result<Listing, Failure> {
    let store = client.store();
    let homes = client.call(Method::Get, "/v1.0/m/life/users/homes", None, None)?;
    let mut devices: Vec<Device> = Vec::new();
    let mut key_changes = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for home in homes.as_array().into_iter().flatten() {
        // The home's `ownerId` is what the device list is keyed by; `id` is
        // the home itself. Some answers carry only one of them.
        let Some(home_id) = home
            .get("ownerId")
            .or_else(|| home.get("id"))
            .map(id_string)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let query = serde_json::json!({ "homeId": home_id });
        let list = client.call(
            Method::Get,
            "/v1.0/m/life/ha/home/devices",
            Some(&query),
            None,
        )?;
        for raw in list.as_array().into_iter().flatten() {
            let text = |k: &str| raw.get(k).and_then(Value::as_str).unwrap_or("");
            let id = text("id").to_string();
            if id.is_empty() || seen.contains(&id) {
                continue;
            }
            seen.push(id.clone());
            // A key can be absent (Bluetooth-only Devices, and some products):
            // then nothing is saved, rather than an empty string.
            let key = text("local_key");
            if !key.is_empty() {
                let changed = store
                    .save_local_key(&id, key, home_store::store::now_unix())
                    .map_err(Failure::storage)?;
                let change = if changed {
                    // `save_local_key` returns true for a new Device and for
                    // a replaced key; tell them apart by the history row.
                    let history = store.local_key_history(&id).map_err(Failure::storage)?;
                    if history.is_empty() {
                        KeyChange::First
                    } else {
                        KeyChange::Changed
                    }
                } else {
                    KeyChange::Unchanged
                };
                key_changes.push((id.clone(), change));
            }
            let category = raw
                .get("category")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            devices.push(Device {
                id,
                name: text("name").to_string(),
                online: raw.get("online").and_then(Value::as_bool).unwrap_or(false),
                category,
                backend: crate::session::BACKEND,
            });
        }
    }
    Ok(Listing {
        devices,
        key_changes,
    })
}

fn id_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// `home tuya keys`: every saved Local Key as JSON, stable Device-ID order.
///
/// This is the one command that prints keys, and it exists so the user can
/// back up the Vendor Exit off the machine.
pub fn local_keys(store: &Store) -> Result<Value, Failure> {
    let records = store.local_keys().map_err(Failure::storage)?;
    let rows: Vec<Value> = records
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "device_id": r.device_id,
                "local_key": r.local_key,
                "first_seen_at": r.first_seen_at,
                "changed_at": r.changed_at,
            })
        })
        .collect();
    Ok(Value::Array(rows))
}
