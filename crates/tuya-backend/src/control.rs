//! `home tuya on|off|get|set <device>`: Cloud Control and Telemetry.
//!
//! A write resolves the Device, is refused for a protected one before any
//! command is sent, and sends the raw command. A read fetches the detail and
//! the declared specification (in that order, spec last so an offline Device
//! that the detail leaves offline is still caught), and splits the points
//! into data points, Telemetry (unit-bearing, not a function, not
//! calibration) and calibration.
//!
//! Spec note: the `/v1.0/m/life/devices/{id}/status` call on Tuya's newer
//! endpoints returns `dpStatusRelationDTOS`, which carries no current values,
//! so this module never calls it; the readings come from the `ha/devices/detail`
//! call instead.

use home_store::{guard, ErrorBody, Exit, Store};
use serde_json::{json, Value};

use crate::devices::Device;
use crate::session::{Failure, Method, TuyaClient, BACKEND};
use crate::Config;

const CALIBRATION: &[&str] = &[
    "voltage_coe",
    "electric_coe",
    "power_coe",
    "electricity_coe",
];

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// `home tuya on|off`: the switch command.
pub fn on_off_command(
    cfg: &Config,
    store: &Store,
    device_ref: &str,
    on: bool,
    force: bool,
) -> Result<Value, (ErrorBody, Exit)> {
    guard::require_with(store, BACKEND, device_ref.trim(), force)?;
    let client = TuyaClient::open(store, cfg).map_err(into_pair)?;
    let device = resolve(&client, device_ref)?;
    guard::require_with(store, BACKEND, &device.id, force)?;
    if !device.online {
        return Err(offline(&device));
    }
    send_command(&client, &device.id, "switch_1", json!(on))?;
    Ok(changed(&device, "switch_1", json!(on)))
}

/// `home tuya get`: data points, Telemetry and calibration.
pub fn get_command(
    cfg: &Config,
    store: &Store,
    device_ref: &str,
) -> Result<Value, (ErrorBody, Exit)> {
    let client = TuyaClient::open(store, cfg).map_err(into_pair)?;
    let device = resolve(&client, device_ref)?;
    if !device.online {
        return Err(offline(&device));
    }
    let detail = detail(&client, &device.id)?;
    let spec = spec(&client, &device.id)?;
    Ok(build_get(&device, &detail, &spec))
}

/// `home tuya set <device> <code> <value>`: one raw data point.
pub fn set_command(
    cfg: &Config,
    store: &Store,
    device_ref: &str,
    code: &str,
    raw_value: &str,
    force: bool,
) -> Result<Value, (ErrorBody, Exit)> {
    guard::require_with(store, BACKEND, device_ref.trim(), force)?;
    let client = TuyaClient::open(store, cfg).map_err(into_pair)?;
    let device = resolve(&client, device_ref)?;
    guard::require_with(store, BACKEND, &device.id, force)?;
    if !device.online {
        return Err(offline(&device));
    }
    let spec = spec(&client, &device.id)?;
    let decl = spec_entry(&spec, code).ok_or_else(|| {
        (
            ErrorBody::usage(format!(
                "no data point `{code}` on device `{}`; check the Device's specification",
                device.id
            )),
            Exit::Usage,
        )
    })?;
    let value = convert(decl, raw_value).ok_or_else(|| {
        (
            ErrorBody::usage(format!(
                "value `{raw_value}` is not a valid {} for `{code}`",
                decl.kind
            )),
            Exit::Usage,
        )
    })?;
    send_command(&client, &device.id, code, value.clone())?;
    Ok(changed(&device, code, value))
}

fn offline(device: &Device) -> (ErrorBody, Exit) {
    (
        ErrorBody::new(
            "device_offline",
            format!("Tuya device `{}` ({}) is offline", device.name, device.id),
        )
        .backend(BACKEND),
        Exit::Offline,
    )
}

fn changed(device: &Device, code: &str, value: Value) -> Value {
    json!({
        "id": device.id,
        "name": device.name,
        "backend": BACKEND,
        "changed": {"code": code, "value": value},
    })
}

fn send_command(
    client: &TuyaClient<'_>,
    device_id: &str,
    code: &str,
    value: Value,
) -> Result<(), (ErrorBody, Exit)> {
    let path = format!("/v1.1/m/thing/{device_id}/commands");
    let body = json!({"commands":[{"code": code, "value": value}]});
    let result = client
        .call(Method::Post, &path, None, Some(&body))
        .map_err(into_pair)?;
    if result.as_bool() != Some(true) {
        return Err((
            ErrorBody::new("upstream", "Tuya refused the command").backend(BACKEND),
            Exit::Upstream,
        ));
    }
    Ok(())
}

type Detail = (bool, Vec<(String, Value)>);

fn detail(client: &TuyaClient<'_>, device_id: &str) -> Result<Detail, (ErrorBody, Exit)> {
    let query = json!({"devIds": device_id});
    let result = client
        .call(
            Method::Get,
            "/v1.0/m/life/ha/devices/detail",
            Some(&query),
            None,
        )
        .map_err(into_pair)?;
    let Some(first) = result.as_array().and_then(|a| a.first()) else {
        return Err((
            ErrorBody::new("upstream", "Tuya returned no detail for this Device").backend(BACKEND),
            Exit::Upstream,
        ));
    };
    let Some(points) = first.get("status").and_then(Value::as_array) else {
        return Err((
            ErrorBody::new("upstream", "Tuya returned no status for this Device").backend(BACKEND),
            Exit::Upstream,
        ));
    };
    let online = first.get("online").and_then(Value::as_bool).unwrap_or(true);
    let entries = points
        .iter()
        .map(|p| {
            (
                p.get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                p.get("value").cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    Ok((online, entries))
}

struct SpecEntry {
    kind: String,
    unit: Option<String>,
    scale: i64,
}

fn spec_entry_value(v: &Value) -> Option<SpecEntry> {
    let values = v.get("values").and_then(Value::as_str).unwrap_or("{}");
    let parsed: Value = serde_json::from_str(values).unwrap_or(json!({}));
    let unit = parsed
        .get("unit")
        .and_then(Value::as_str)
        .map(str::to_string);
    let scale = parsed
        .get("scale")
        .and_then(|s| {
            s.as_i64()
                .or_else(|| s.as_u64().and_then(|u| i64::try_from(u).ok()))
        })
        .unwrap_or(0);
    let kind = v
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("String")
        .to_string();
    Some(SpecEntry { kind, unit, scale })
}

fn spec(client: &TuyaClient<'_>, device_id: &str) -> Result<Spec, (ErrorBody, Exit)> {
    let path = format!("/v1.1/m/life/{device_id}/specifications");
    let result = client
        .call(Method::Get, &path, None, None)
        .map_err(into_pair)?;
    let Some(status) = result.get("status").and_then(Value::as_array) else {
        return Err((
            ErrorBody::new("upstream", "Tuya returned no specification for this Device")
                .backend(BACKEND),
            Exit::Upstream,
        ));
    };
    let status: Vec<(String, SpecEntry)> = status
        .iter()
        .filter_map(|v| {
            let code = v.get("code").and_then(Value::as_str)?.to_string();
            Some((code, spec_entry_value(v)?))
        })
        .collect();
    let function_rows = result
        .get("functions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let functions: Vec<String> = function_rows
        .iter()
        .filter_map(|v| v.get("code").and_then(Value::as_str).map(str::to_string))
        .collect();
    let function_entries: Vec<(String, SpecEntry)> = function_rows
        .iter()
        .filter_map(|v| {
            let code = v.get("code").and_then(Value::as_str)?.to_string();
            Some((code, spec_entry_value(v)?))
        })
        .collect();
    Ok(Spec {
        status,
        functions,
        function_entries,
    })
}

struct Spec {
    /// `result.status[]` entries, in order.
    status: Vec<(String, SpecEntry)>,
    /// `result.functions[]` codes, in order.
    functions: Vec<String>,
    /// `result.functions[]` entries, for lookups when status lacks the code.
    function_entries: Vec<(String, SpecEntry)>,
}

impl Spec {
    fn entry(&self, code: &str) -> Option<&SpecEntry> {
        self.status
            .iter()
            .find(|(c, _)| c == code)
            .or_else(|| self.function_entries.iter().find(|(c, _)| c == code))
            .map(|(_, e)| e)
    }
}

fn spec_entry<'a>(spec: &'a Spec, code: &str) -> Option<&'a SpecEntry> {
    spec.entry(code)
}

fn type_of(spec: &Spec, code: &str) -> Option<String> {
    spec.entry(code).map(|e| e.kind.clone())
}

fn convert(entry: &SpecEntry, raw: &str) -> Option<Value> {
    match entry.kind.as_str() {
        "Boolean" => match raw.trim().to_ascii_lowercase().as_str() {
            "true" => Some(json!(true)),
            "false" => Some(json!(false)),
            _ => None,
        },
        "Integer" => raw.trim().parse::<i64>().ok().map(Value::from),
        _ => Some(json!(raw)),
    }
}

fn build_get(device: &Device, detail: &Detail, spec: &Spec) -> Value {
    let (online, points) = detail;
    let mut data_points = Vec::new();
    let mut telemetry = serde_json::Map::new();
    let mut calibration = serde_json::Map::new();
    for (code, raw) in points {
        let mut dp = serde_json::Map::new();
        dp.insert("code".into(), json!(code));
        dp.insert("value".into(), raw.clone());
        if let Some(t) = type_of(spec, code) {
            dp.insert("type".into(), json!(t));
        }
        data_points.push(Value::Object(dp));
        let decl = spec.entry(code);
        if let Some(entry) = decl {
            if CALIBRATION.contains(&code.as_str()) {
                calibration.insert(code.clone(), raw.clone());
            } else if let Some(unit) = &entry.unit {
                if !spec.functions.contains(code) {
                    let scale = entry.scale;
                    let value = if scale == 0 {
                        raw.clone()
                    } else {
                        let raw_f = match raw {
                            Value::Number(n) => n.as_f64().unwrap_or(0.0),
                            _ => 0.0,
                        };
                        let divisor = 10f64.powi(scale as i32);
                        json!(raw_f / divisor)
                    };
                    telemetry.insert(
                        code.clone(),
                        json!({"raw": raw, "scale": scale, "value": value, "unit": unit}),
                    );
                }
            }
        }
    }
    json!({
        "id": device.id,
        "name": device.name,
        "online": online,
        "backend": BACKEND,
        "data_points": data_points,
        "telemetry": Value::Object(telemetry),
        "calibration": Value::Object(calibration),
    })
}

// ---------------------------------------------------------------------------
// Device resolution, ticket-03 rules

fn resolve(client: &TuyaClient<'_>, reference: &str) -> Result<Device, (ErrorBody, Exit)> {
    let listing = crate::devices::list_devices(client).map_err(into_pair)?;
    resolve_in(&listing.devices, reference)
}

fn resolve_in(devices: &[Device], reference: &str) -> Result<Device, (ErrorBody, Exit)> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Err((
            ErrorBody::usage("a Device reference is empty; pass an ID or name"),
            Exit::Usage,
        ));
    }
    if let Some(device) = devices.iter().find(|d| d.id == reference) {
        return Ok(device.clone());
    }
    let name = lower(reference);
    let matches: Vec<&Device> = devices.iter().filter(|d| lower(&d.name) == name).collect();
    match matches.len() {
        0 => Err((
            ErrorBody::usage(format!(
                "no Tuya Device matches `{reference}`; run `home tuya devices` for the list"
            ))
            .backend(BACKEND),
            Exit::Usage,
        )),
        1 => Ok(matches[0].clone()),
        _ => Err(ambiguous(reference, &matches)),
    }
}

fn ambiguous(reference: &str, matches: &[&Device]) -> (ErrorBody, Exit) {
    let candidates = matches
        .iter()
        .map(|d| serde_json::to_string(d).unwrap_or_default())
        .collect();
    let list = matches
        .iter()
        .map(|d| format!("{} ({})", d.name, d.id))
        .collect::<Vec<_>>()
        .join(", ");
    (
        ErrorBody::new(
            "ambiguous",
            format!("`{reference}` matches {} devices: {list}", matches.len()),
        )
        .backend(BACKEND)
        .candidates(candidates),
        Exit::Ambiguous,
    )
}

fn into_pair(f: Failure) -> (ErrorBody, Exit) {
    (f.body, f.exit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, name: &str) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            online: true,
            category: Some("cz".into()),
            backend: BACKEND,
        }
    }

    fn devices() -> Vec<Device> {
        vec![
            dev("tuyaFakeHome000000001", "Home"),
            dev("tuyaFakeLab0000000002", "Plug"),
            dev("tuyaFakeLamp000000003", "Plug"),
            dev("tuyaFakeSkew000000003", "Skew"),
        ]
    }

    #[test]
    fn an_exact_id_wins_over_a_name_that_spells_it() {
        let devices = vec![dev("id-plug", "Plug"), dev("other", "id-plug")];
        assert_eq!(resolve_in(&devices, "id-plug").unwrap().id, "id-plug");
    }

    #[test]
    fn resolution_is_case_insensitive_on_names() {
        assert_eq!(
            resolve_in(&devices(), "home").unwrap().id,
            "tuyaFakeHome000000001"
        );
        assert_eq!(
            resolve_in(&devices(), "HOME").unwrap().id,
            "tuyaFakeHome000000001"
        );
    }

    #[test]
    fn a_repeated_name_is_ambiguous_with_json_candidates() {
        let (body, exit) = resolve_in(&devices(), "Plug").unwrap_err();
        assert_eq!(exit.code(), 3);
        assert_eq!(body.code, "ambiguous");
        assert_eq!(body.backend, Some("tuya"));
        assert_eq!(body.candidates.len(), 2);
        let first: Value = serde_json::from_str(&body.candidates[0]).unwrap();
        assert_eq!(first["id"], "tuyaFakeLab0000000002");
        assert_eq!(first["name"], "Plug");
        assert!(first.get("category").is_some());
        assert_eq!(first["backend"], "tuya");
        assert!(body.message.contains("tuyaFakeLab0000000002"));
        assert!(body.message.contains("tuyaFakeLamp000000003"));
    }

    #[test]
    fn an_unknown_or_empty_reference_is_usage() {
        assert_eq!(resolve_in(&devices(), "Nope").unwrap_err().1.code(), 2);
        assert_eq!(resolve_in(&devices(), "  ").unwrap_err().1.code(), 2);
        let (body, _) = resolve_in(&devices(), "Nope").unwrap_err();
        assert!(body.message.contains("home tuya devices"));
    }

    #[test]
    fn the_third_device_does_not_hinder_an_exact_id() {
        assert_eq!(
            resolve_in(&devices(), "tuyaFakeSkew000000003")
                .unwrap()
                .name,
            "Skew"
        );
    }
}
