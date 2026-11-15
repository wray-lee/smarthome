//! Codec for `StructuresService/GetHomeGraph`: positional JSON arrays
//! (protobuf field number N → index N-1) become typed homes, rooms and
//! Devices. Every accessor is total, so a Google-side shape change empties a
//! column instead of aborting the command.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Home {
    pub id: String,
    pub name: String,
    pub rooms: Vec<Room>,
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub device_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub traits: Vec<String>,
    pub room: Option<String>,
    /// The Linked Service's id (for example `tuya-smart-cee31`). Only
    /// `UpdateTraits` needs it, so it is skipped in the public JSON.
    #[serde(skip_serializing)]
    pub agent_id: Option<String>,
    /// The Linked Service's own id for the Device (`…_all`). Internal, like
    /// `agent_id`: an `UpdateTraits` entry must carry both.
    #[serde(skip_serializing)]
    pub partner_device_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HomeGraph {
    pub homes: Vec<Home>,
}

fn at(v: &Value, i: usize) -> &Value {
    v.get(i).unwrap_or(&Value::Null)
}

fn string(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn parse_room(v: &Value) -> Option<Room> {
    let id = string(at(v, 0))?;
    let device_ids = at(v, 4)
        .as_array()
        .map(|members| {
            members
                .iter()
                .filter_map(|m| string(at(at(m, 0), 0)))
                .collect()
        })
        .unwrap_or_default();
    Some(Room {
        id,
        name: string(at(v, 2)).unwrap_or_default(),
        device_ids,
    })
}

fn parse_device(v: &Value) -> Option<Device> {
    let id = string(at(at(v, 0), 0))?;
    let partner = at(v, 0).get(1);
    Some(Device {
        id,
        name: string(at(v, 3)).unwrap_or_default(),
        kind: string(at(v, 5)),
        traits: at(v, 6)
            .as_array()
            .map(|a| a.iter().filter_map(string).collect())
            .unwrap_or_default(),
        room: None,
        agent_id: partner.and_then(|p| string(at(p, 0))),
        partner_device_id: partner.and_then(|p| string(at(p, 1))),
    })
}

fn parse_home(v: &Value) -> Option<Home> {
    let id = string(at(v, 0))?;
    let rooms: Vec<Room> = at(v, 5)
        .as_array()
        .map(|a| a.iter().filter_map(parse_room).collect())
        .unwrap_or_default();
    let mut devices: Vec<Device> = at(v, 6)
        .as_array()
        .map(|a| a.iter().filter_map(parse_device).collect())
        .unwrap_or_default();
    for d in &mut devices {
        if let Some(r) = rooms.iter().find(|r| r.device_ids.contains(&d.id)) {
            d.room = Some(r.name.clone());
        }
    }
    Some(Home {
        id,
        name: string(at(v, 1)).unwrap_or_default(),
        rooms,
        devices,
    })
}

/// Parse a raw `GetHomeGraph` response. Home slot: field 2, which arrives as
/// one home on single-home accounts and a list on multi-home ones.
pub fn parse(raw: &Value) -> HomeGraph {
    let slot = at(raw, 1);
    let homes = match slot.as_array() {
        Some(a) if a.first().map(Value::is_string).unwrap_or(false) => {
            parse_home(slot).into_iter().collect()
        }
        Some(a) => a.iter().filter_map(parse_home).collect(),
        None => Vec::new(),
    };
    HomeGraph { homes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn homes_rooms_and_membership_parse() {
        let g = parse(&json!([
            1700000000,
            [[
                "home-1",
                "My Home",
                null,
                null,
                null,
                [[
                    "room-office",
                    null,
                    "Office",
                    ["OFFICE"],
                    [[["dev-1", ["agent-a", "P1"]]]]
                ]],
                [
                    [
                        ["dev-1", ["agent-a", "P1"]],
                        null,
                        null,
                        "Office Lamp",
                        null,
                        "action.devices.types.LIGHT",
                        ["action.devices.traits.OnOff"]
                    ],
                    [
                        ["dev-2", ["agent-b", "P2"]],
                        null,
                        null,
                        "Lonely Plug",
                        null,
                        "action.devices.types.OUTLET",
                        []
                    ]
                ]
            ]]
        ]));
        assert_eq!(g.homes.len(), 1);
        let h = &g.homes[0];
        assert_eq!(h.name, "My Home");
        assert_eq!(h.rooms.len(), 1);
        assert_eq!(h.devices.len(), 2);
        assert_eq!(h.devices[0].room.as_deref(), Some("Office"));
        assert!(h.devices[1].room.is_none());
        assert_eq!(h.devices[0].traits, ["action.devices.traits.OnOff"]);
    }

    #[test]
    fn a_single_home_slot_is_accepted() {
        let g = parse(&json!([0, ["home-x", "Solo", null, null, null, [], []]]));
        assert_eq!(g.homes.len(), 1);
        assert_eq!(g.homes[0].id, "home-x");
    }

    #[test]
    fn garbage_parses_to_empty_not_panic() {
        assert!(parse(&json!(null)).homes.is_empty());
        assert!(parse(&json!([1, "nope"])).homes.is_empty());
        assert!(parse(&json!([1, [[42]]])).homes.is_empty());
    }
}
