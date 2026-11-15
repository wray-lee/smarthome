//! Turning what an agent typed into exactly one Device.
//!
//! Order, per the spec: an exact Device ID first, then `room/name`, then the
//! bare name. A reference that fits several Devices is never guessed: it
//! comes back as exit 3 with every candidate listed.

use home_store::{ErrorBody, Exit};

use crate::Device;

fn lower(s: &str) -> String {
    s.to_lowercase()
}

fn candidate(device: &Device) -> String {
    serde_json::json!({
        "id": device.id,
        "name": device.name,
        "room": device.room,
        "kind": device.kind,
    })
    .to_string()
}

fn ambiguous(reference: &str, matches: &[&Device]) -> (ErrorBody, Exit) {
    let candidates: Vec<String> = matches.iter().map(|d| candidate(d)).collect();
    let list = matches
        .iter()
        .map(|d| match &d.room {
            Some(room) => format!("{room}/{}{}", d.name, format_args!(" ({})", d.id)),
            None => format!("{} ({})", d.name, d.id),
        })
        .collect::<Vec<_>>()
        .join(", ");
    (
        ErrorBody::new(
            "ambiguous",
            format!("`{reference}` matches {} devices: {list}", matches.len()),
        )
        .backend(crate::BACKEND)
        .candidates(candidates),
        Exit::Ambiguous,
    )
}

/// One Device out of `devices`, or the exit-3 / exit-2 error.
pub fn resolve(devices: &[Device], reference: &str) -> Result<Device, (ErrorBody, Exit)> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Err((
            ErrorBody::usage("a Device reference is empty; pass an ID, room/name, or name"),
            Exit::Usage,
        ));
    }
    // 1. Exact ID. It wins even when another Device's name spells the same
    // string, so an agent that has an ID always reaches that Device.
    if let Some(device) = devices.iter().find(|d| d.id == reference) {
        return Ok(device.clone());
    }
    // 2. room/name, case-insensitive on both halves.
    let mut matches: Vec<&Device> = Vec::new();
    if let Some((room, name)) = reference.split_once('/') {
        let room = lower(room.trim());
        let name = lower(name.trim());
        matches = devices
            .iter()
            .filter(|d| {
                d.room.as_deref().map(lower).is_some_and(|r| r == room) && lower(&d.name) == name
            })
            .collect();
    }
    // 3. Bare name.
    if matches.is_empty() {
        let name = lower(reference);
        matches = devices.iter().filter(|d| lower(&d.name) == name).collect();
    }
    match matches.len() {
        0 => Err((
            ErrorBody::usage(format!(
                "no Google Home Device matches `{reference}`; run `home gh devices` for the list"
            ))
            .backend(crate::BACKEND),
            Exit::Usage,
        )),
        1 => Ok(matches[0].clone()),
        _ => Err(ambiguous(reference, &matches)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, name: &str, room: Option<&str>) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            kind: Some("action.devices.types.OUTLET".into()),
            traits: vec!["action.devices.traits.OnOff".into()],
            room: room.map(str::to_string),
            agent_id: Some("agent".into()),
            partner_device_id: Some("partner".into()),
            backend: crate::BACKEND,
        }
    }

    fn devices() -> Vec<Device> {
        vec![
            dev("id-plug", "Plug", Some("主卧")),
            dev("id-lamp", "Lamp", Some("书房")),
            dev("id-lamp-2", "Lamp", Some("主卧")),
            dev("id-office", "Desk", Some("Office")),
            dev("id-lonely", "Lonely", None),
            // A Device whose name spells another Device's ID.
            dev("id-imposter", "id-plug", Some("书房")),
        ]
    }

    #[test]
    fn an_exact_id_wins_over_a_name_that_spells_it() {
        let d = resolve(&devices(), "id-plug").unwrap();
        assert_eq!(d.id, "id-plug");
        assert_eq!(d.name, "Plug");
    }

    #[test]
    fn room_and_name_resolve_case_insensitively() {
        assert_eq!(resolve(&devices(), "书房/Lamp").unwrap().id, "id-lamp");
        assert_eq!(resolve(&devices(), "书房/lamp").unwrap().id, "id-lamp");
        assert_eq!(resolve(&devices(), "office/desk").unwrap().id, "id-office");
        // Chinese room names survive the lowercase round trip.
        assert_eq!(resolve(&devices(), "主卧/Plug").unwrap().id, "id-plug");
        // Whitespace around the halves is ignored.
        assert_eq!(resolve(&devices(), " 主卧 / Plug ").unwrap().id, "id-plug");
    }

    #[test]
    fn a_bare_name_resolves_when_it_is_unique() {
        assert_eq!(resolve(&devices(), "Desk").unwrap().id, "id-office");
        assert_eq!(resolve(&devices(), "desk").unwrap().id, "id-office");
        assert_eq!(resolve(&devices(), "Lonely").unwrap().id, "id-lonely");
    }

    #[test]
    fn a_repeated_name_is_ambiguous_with_json_candidates() {
        let (body, exit) = resolve(&devices(), "Lamp").unwrap_err();
        assert_eq!(exit.code(), 3);
        assert_eq!(body.code, "ambiguous");
        assert_eq!(body.backend, Some("google_home"));
        assert_eq!(body.candidates.len(), 2);
        let first: serde_json::Value = serde_json::from_str(&body.candidates[0]).unwrap();
        assert_eq!(first["id"], "id-lamp");
        assert_eq!(first["name"], "Lamp");
        assert_eq!(first["room"], "书房");
        assert_eq!(first["kind"], "action.devices.types.OUTLET");
        let second: serde_json::Value = serde_json::from_str(&body.candidates[1]).unwrap();
        assert_eq!(second["id"], "id-lamp-2");
        // The prose names both rooms so a human can choose too.
        assert!(body.message.contains("书房"), "{}", body.message);
        assert!(body.message.contains("主卧"), "{}", body.message);
    }

    #[test]
    fn room_and_name_that_match_nothing_fall_back_to_the_bare_name() {
        // `主卧/Lamp` matches one Device; `nowhere/Lamp` matches none.
        let (body, exit) = resolve(&devices(), "nowhere/Lamp").unwrap_err();
        assert_eq!(exit.code(), 2);
        assert_eq!(body.code, "usage");
        assert!(body.message.contains("nowhere/Lamp"));
    }

    #[test]
    fn an_unknown_or_empty_reference_is_usage() {
        assert_eq!(resolve(&devices(), "Nope").unwrap_err().1.code(), 2);
        assert_eq!(resolve(&devices(), "  ").unwrap_err().1.code(), 2);
        assert_eq!(resolve(&[], "anything").unwrap_err().1.code(), 2);
    }
}
