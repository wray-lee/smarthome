//! `HomeControlService/GetTraits` and `UpdateTraits`: reading and changing a
//! Device's current traits.
//!
//! Both RPCs speak the same positional-array codec as `GetHomeGraph`. A trait
//! payload is `[traitName, [[fieldName, wrapper], …]]`, where the wrapper puts
//! the value at a type-specific slot: an int at index 1 (`[null, n]`), a string
//! at index 2 (`[null, null, "s"]`), a bool at index 3
//! (`[null, null, null, 0|1]`).
//!
//! Layouts follow the fork's upstream codec (`tmp/ghome-src/src/traits.rs`) and
//! the ticket-01 capture in `.scratch/smarthome-v1/fixtures/foyer/`.

use serde_json::{json, Map, Value};

pub const GET_TRAITS: &str = "GetTraits";
pub const UPDATE_TRAITS: &str = "UpdateTraits";

fn at(v: &Value, i: usize) -> &Value {
    v.get(i).unwrap_or(&Value::Null)
}

fn string(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

/// Unwrap a scalar wrapper into a plain JSON value.
pub fn unwrap_scalar(w: &Value) -> Value {
    if let Some(n) = at(w, 1).as_i64() {
        return json!(n);
    }
    if let Some(f) = at(w, 1).as_f64() {
        return json!(f);
    }
    if let Some(s) = at(w, 2).as_str() {
        return json!(s);
    }
    match at(w, 3) {
        Value::Number(n) => json!(n.as_i64() == Some(1)),
        Value::Bool(b) => json!(*b),
        _ => Value::Null,
    }
}

/// The `GetTraits` request body: `[[["id1"], ["id2"], …]]`.
pub fn get_traits_body(ids: &[String]) -> Value {
    json!([ids.iter().map(|id| json!([id])).collect::<Vec<Value>>()])
}

/// Parse a `GetTraits` or `UpdateTraits` answer into
/// `{deviceId: {trait: {field: value}}}`.
pub fn parse_states(raw: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    for entry in at(raw, 0).as_array().into_iter().flatten() {
        let Some(id) = string(at(at(entry, 0), 0)) else {
            continue;
        };
        let mut traits = Map::new();
        for trait_ in at(entry, 1).as_array().into_iter().flatten() {
            let Some(name) = string(at(trait_, 0)) else {
                continue;
            };
            let mut fields = Map::new();
            for field in at(trait_, 1).as_array().into_iter().flatten() {
                if let Some(field_name) = string(at(field, 0)) {
                    let value = unwrap_scalar(at(field, 1));
                    if !value.is_null() {
                        fields.insert(field_name, value);
                    }
                }
            }
            traits.insert(name, Value::Object(fields));
        }
        out.insert(id, Value::Object(traits));
    }
    out
}

/// One Device's traits out of a control answer; empty when the answer does
/// not mention the Device.
pub fn traits_for(raw: &Value, id: &str) -> Map<String, Value> {
    parse_states(raw)
        .remove(id)
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// `deviceStatus.online`, when the Device reported it.
pub fn online(traits: &Map<String, Value>) -> Option<bool> {
    traits.get("deviceStatus")?.get("online")?.as_bool()
}

/// The JSON `home gh get` prints for one Device: identity plus every trait the
/// foyer answer carried. Device metadata uses `kind`, matching `home gh
/// devices`.
pub fn state_document(
    id: &str,
    name: &str,
    room: Option<&str>,
    kind: Option<&str>,
    traits: &Map<String, Value>,
    online: Option<bool>,
) -> Value {
    let mut doc = Map::new();
    doc.insert("id".into(), json!(id));
    doc.insert("name".into(), json!(name));
    if let Some(room) = room {
        doc.insert("room".into(), json!(room));
    }
    if let Some(kind) = kind {
        doc.insert("kind".into(), json!(kind));
    }
    doc.insert("backend".into(), json!(crate::BACKEND));
    if let Some(online) = online {
        doc.insert("online".into(), json!(online));
    }
    doc.insert("traits".into(), Value::Object(traits.clone()));
    Value::Object(doc)
}

/// What a `set` asks for. One variant per trait family the CLI exposes.
///
/// `Lock(false)` is an unlock and `ArmDisarm(false)` a disarm; those two are
/// the Sensitive Actions that need `--confirm`.
#[derive(Debug, Clone, PartialEq)]
pub enum TraitChange {
    OnOff(bool),
    /// 0–100.
    Brightness(u8),
    /// Kelvin.
    ColorTemperature(u32),
    /// 0–100.
    Volume(u8),
    /// `true` locks, `false` unlocks.
    Lock(bool),
    /// `true` arms, `false` disarms.
    ArmDisarm(bool),
    /// Mode and setpoint always travel together.
    Thermostat {
        mode: String,
        setpoint: f64,
    },
}

fn usage(message: impl Into<String>) -> (home_store::ErrorBody, home_store::Exit) {
    (
        home_store::ErrorBody::usage(message),
        home_store::Exit::Usage,
    )
}

fn bool_value(value: &str) -> Result<bool, (home_store::ErrorBody, home_store::Exit)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "1" | "yes" => Ok(true),
        "false" | "off" | "0" | "no" => Ok(false),
        other => Err(usage(format!(
            "`{other}` is not a boolean; use true or false"
        ))),
    }
}

fn percent(value: &str) -> Result<u8, (home_store::ErrorBody, home_store::Exit)> {
    let n: u8 = value
        .trim()
        .parse()
        .map_err(|_| usage(format!("`{value}` is not a number from 0 to 100")))?;
    if n > 100 {
        return Err(usage(format!("{n} is outside 0–100")));
    }
    Ok(n)
}

impl TraitChange {
    /// The names the CLI accepts for `home gh set <device> <trait> <value>`.
    pub const TRAITS: &'static [&'static str] = &[
        "onOff",
        "brightness",
        "color.colorTemperature",
        "volume.currentVolume",
        "lockUnlock.isLocked",
        "armDisarm.isArmed",
        "thermostat",
    ];

    /// Parse the CLI's `<trait> <value> [--mode M] [--setpoint S]` form.
    ///
    /// A thermostat needs both a mode and a setpoint. Either may come from
    /// the flags, and `<value>` may repeat one of them (`thermostat heat
    /// --setpoint 21`, or `thermostat 21 --mode heat`).
    pub fn parse(
        trait_name: &str,
        value: &str,
        mode: Option<&str>,
        setpoint: Option<f64>,
    ) -> Result<Self, (home_store::ErrorBody, home_store::Exit)> {
        let name = trait_name.trim().to_ascii_lowercase();
        match name.as_str() {
            "onoff" | "on_off" => Ok(TraitChange::OnOff(bool_value(value)?)),
            "brightness" => Ok(TraitChange::Brightness(percent(value)?)),
            "color" | "colortemperature" | "color.colortemperature" | "color_temperature" => {
                let kelvin: u32 = value.trim().parse().map_err(|_| {
                    usage(format!("`{value}` is not a colour temperature in kelvin"))
                })?;
                if !(1000..=10000).contains(&kelvin) {
                    return Err(usage(format!(
                        "{kelvin} K is outside 1000–10000 K; pass a colour temperature in kelvin"
                    )));
                }
                Ok(TraitChange::ColorTemperature(kelvin))
            }
            "volume" | "volume.currentvolume" | "currentvolume" => {
                Ok(TraitChange::Volume(percent(value)?))
            }
            "lockunlock" | "lockunlock.islocked" | "islocked" => {
                Ok(TraitChange::Lock(bool_value(value)?))
            }
            "armdisarm" | "armdisarm.isarmed" | "isarmed" => {
                Ok(TraitChange::ArmDisarm(bool_value(value)?))
            }
            "thermostat" | "temperaturesetting" | "temperature" => {
                let flag_mode = mode.map(str::trim).filter(|m| !m.is_empty());
                let value_number = value.trim().parse::<f64>().ok();
                let mode = match (flag_mode, value_number) {
                    (Some(mode), _) => Some(mode.to_string()),
                    (None, Some(_)) => None,
                    (None, None) => Some(value.trim().to_string()).filter(|v| !v.is_empty()),
                };
                let setpoint = setpoint.or(value_number);
                match (mode, setpoint) {
                    (Some(mode), Some(setpoint)) => Ok(TraitChange::Thermostat { mode, setpoint }),
                    _ => Err(usage(
                        "a thermostat change needs both a mode and a setpoint: pass \
                         `--mode heat --setpoint 21` (or put one of them in <value>)",
                    )),
                }
            }
            other => Err(usage(format!(
                "unknown trait `{other}`; supported: {}",
                Self::TRAITS.join(", ")
            ))),
        }
    }

    /// Does this change weaken physical security?
    pub fn is_sensitive(&self) -> bool {
        matches!(
            self,
            TraitChange::Lock(false) | TraitChange::ArmDisarm(false)
        )
    }

    /// How the change reads in an error or a confirmation message.
    pub fn action_name(&self) -> String {
        match self {
            TraitChange::OnOff(true) => "turning on".into(),
            TraitChange::OnOff(false) => "turning off".into(),
            TraitChange::Brightness(b) => format!("setting brightness to {b}"),
            TraitChange::ColorTemperature(k) => format!("setting colour temperature to {k} K"),
            TraitChange::Volume(v) => format!("setting volume to {v}"),
            TraitChange::Lock(true) => "locking".into(),
            TraitChange::Lock(false) => "unlocking".into(),
            TraitChange::ArmDisarm(true) => "arming".into(),
            TraitChange::ArmDisarm(false) => "disarming".into(),
            TraitChange::Thermostat { mode, setpoint } => {
                format!("setting thermostat to {mode} at {setpoint}")
            }
        }
    }

    /// The change as the `changed` field of the `set` document.
    pub fn as_json(&self) -> Value {
        match self {
            TraitChange::OnOff(v) => json!({"trait": "onOff", "value": v}),
            TraitChange::Brightness(v) => json!({"trait": "brightness", "value": v}),
            TraitChange::ColorTemperature(v) => {
                json!({"trait": "color.colorTemperature", "value": v})
            }
            TraitChange::Volume(v) => json!({"trait": "volume.currentVolume", "value": v}),
            TraitChange::Lock(v) => json!({"trait": "lockUnlock.isLocked", "value": v}),
            TraitChange::ArmDisarm(v) => json!({"trait": "armDisarm.isArmed", "value": v}),
            TraitChange::Thermostat { mode, setpoint } => json!({
                "trait": "thermostat",
                "mode": mode,
                "setpoint": setpoint,
            }),
        }
    }

    /// The Google trait this change needs on the Device, as a suffix match
    /// (`action.devices.traits.<name>`).
    pub fn required_trait(&self) -> &'static str {
        match self {
            TraitChange::OnOff(_) => "OnOff",
            TraitChange::Brightness(_) => "Brightness",
            TraitChange::ColorTemperature(_) => "ColorSetting",
            TraitChange::Volume(_) => "Volume",
            TraitChange::Lock(_) => "LockUnlock",
            TraitChange::ArmDisarm(_) => "ArmDisarm",
            TraitChange::Thermostat { .. } => "TemperatureSetting",
        }
    }

    /// The trait entries of one `UpdateTraits` device block.
    pub fn entries(&self) -> Vec<Value> {
        match self {
            TraitChange::OnOff(on) => {
                vec![json!(["onOff", [["onOff", bool_w(*on)]]])]
            }
            TraitChange::Brightness(b) => vec![json!([
                "brightness",
                [["brightness", int_w(i64::from(*b))]]
            ])],
            TraitChange::ColorTemperature(k) => vec![json!([
                "color",
                [["colorTemperature", int_w(i64::from(*k))]]
            ])],
            TraitChange::Volume(v) => {
                vec![json!(["volume", [["currentVolume", int_w(i64::from(*v))]]])]
            }
            TraitChange::Lock(locked) => {
                vec![json!(["lockUnlock", [["isLocked", bool_w(*locked)]]])]
            }
            TraitChange::ArmDisarm(armed) => {
                vec![json!(["armDisarm", [["isArmed", bool_w(*armed)]]])]
            }
            TraitChange::Thermostat { mode, setpoint } => vec![json!([
                "temperatureSetting",
                [
                    ["mode", str_w(mode)],
                    [
                        "thermostatTemperatureSetpoint",
                        int_w(setpoint.round() as i64)
                    ]
                ]
            ])],
        }
    }
}

fn bool_w(b: bool) -> Value {
    json!([null, null, null, if b { 1 } else { 0 }])
}

fn int_w(n: i64) -> Value {
    json!([null, n])
}

fn str_w(s: &str) -> Value {
    json!([null, null, s])
}

/// One `UpdateTraits` request body:
/// `[[ [[id, [agentId, partnerId]], [trait, …]], … ]]`.
pub fn update_body(
    id: &str,
    agent_id: Option<&str>,
    partner_device_id: Option<&str>,
    change: &TraitChange,
) -> Value {
    let entry = json!([
        [
            id,
            [
                agent_id.unwrap_or_default(),
                partner_device_id.unwrap_or_default()
            ]
        ],
        change.entries()
    ]);
    json!([[entry]])
}

/// Does the Device advertise the trait this change needs?
pub fn supports(device_traits: &[String], change: &TraitChange) -> bool {
    let suffix = change.required_trait();
    device_traits.iter().any(|t| t.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME_ID: &str = "00000000-0000-4000-8000-00000000h0me";

    fn fixture_answer() -> Value {
        json!([[[
            [HOME_ID],
            [
                [
                    "deviceStatus",
                    [
                        ["online", [null, null, null, 1]],
                        ["onlineStateDetails", [null, null, "stateOnline"]]
                    ]
                ],
                ["onOff", [["onOff", [null, null, null, 0]]]],
                ["brightness", [["brightness", [null, 40]]]],
                ["color", [["colorTemperature", [null, 2700]]]],
                ["volume", [["currentVolume", [null, 30]]]],
                ["lockUnlock", [["isLocked", [null, null, null, 1]]]],
                [
                    "temperatureSetting",
                    [
                        ["mode", [null, null, "heat"]],
                        ["thermostatTemperatureSetpoint", [null, 21]]
                    ]
                ]
            ]
        ]]])
    }

    #[test]
    fn traits_parse_from_the_captured_shape() {
        let states = parse_states(&fixture_answer());
        let traits = states.get(HOME_ID).unwrap().as_object().unwrap().clone();
        assert_eq!(online(&traits), Some(true));
        assert_eq!(traits["onOff"]["onOff"], false);
        assert_eq!(traits["brightness"]["brightness"], 40);
        assert_eq!(traits["color"]["colorTemperature"], 2700);
        assert_eq!(traits["volume"]["currentVolume"], 30);
        assert_eq!(traits["lockUnlock"]["isLocked"], true);
        assert_eq!(traits["temperatureSetting"]["mode"], "heat");
        assert_eq!(
            traits["temperatureSetting"]["thermostatTemperatureSetpoint"],
            21
        );
    }

    #[test]
    fn an_offline_device_is_reported_as_offline() {
        let raw = json!([[[
            [HOME_ID],
            [["deviceStatus", [["online", [null, null, null, 0]]]]]
        ]]]);
        let traits = traits_for(&raw, HOME_ID);
        assert_eq!(online(&traits), Some(false));
        // A Device the answer omits has no trait map and no online flag.
        assert!(traits_for(&raw, "other").is_empty());
        assert_eq!(online(&Map::new()), None);
    }

    #[test]
    fn get_traits_body_wraps_each_id_in_its_own_list() {
        assert_eq!(
            get_traits_body(&[HOME_ID.to_string(), "dev-2".to_string()]),
            json!([[[HOME_ID], ["dev-2"]]])
        );
    }

    #[test]
    fn bool_int_and_string_wrappers_sit_in_their_slots() {
        assert_eq!(bool_w(true), json!([null, null, null, 1]));
        assert_eq!(bool_w(false), json!([null, null, null, 0]));
        assert_eq!(int_w(40), json!([null, 40]));
        assert_eq!(str_w("heat"), json!([null, null, "heat"]));
        assert_eq!(unwrap_scalar(&int_w(40)), json!(40));
        assert_eq!(unwrap_scalar(&str_w("heat")), json!("heat"));
        assert_eq!(unwrap_scalar(&bool_w(true)), json!(true));
        assert_eq!(unwrap_scalar(&Value::Null), Value::Null);
    }

    #[test]
    fn each_change_makes_the_expected_trait_entry() {
        let one = |c: TraitChange| c.entries();
        assert_eq!(
            one(TraitChange::OnOff(false)),
            vec![json!(["onOff", [["onOff", [null, null, null, 0]]]])]
        );
        assert_eq!(
            one(TraitChange::Brightness(75)),
            vec![json!(["brightness", [["brightness", [null, 75]]]])]
        );
        assert_eq!(
            one(TraitChange::ColorTemperature(2700)),
            vec![json!(["color", [["colorTemperature", [null, 2700]]]])]
        );
        assert_eq!(
            one(TraitChange::Volume(30)),
            vec![json!(["volume", [["currentVolume", [null, 30]]]])]
        );
        assert_eq!(
            one(TraitChange::Lock(true)),
            vec![json!(["lockUnlock", [["isLocked", [null, null, null, 1]]]])]
        );
        assert_eq!(
            one(TraitChange::ArmDisarm(false)),
            vec![json!(["armDisarm", [["isArmed", [null, null, null, 0]]]])]
        );
        // Mode and setpoint are one trait entry, never two.
        assert_eq!(
            one(TraitChange::Thermostat {
                mode: "heat".into(),
                setpoint: 21.4,
            }),
            vec![json!([
                "temperatureSetting",
                [
                    ["mode", [null, null, "heat"]],
                    ["thermostatTemperatureSetpoint", [null, 21]]
                ]
            ])]
        );
    }

    #[test]
    fn the_update_body_carries_the_agent_and_partner_ids() {
        let change = TraitChange::OnOff(true);
        assert_eq!(
            update_body(
                HOME_ID,
                Some("tuya-smart-cee31"),
                Some("tuyaFakeHome000000001_all"),
                &change
            ),
            json!([[[
                [HOME_ID, ["tuya-smart-cee31", "tuyaFakeHome000000001_all"]],
                [["onOff", [["onOff", [null, null, null, 1]]]]]
            ]]])
        );
        // Missing ids degrade to empty strings rather than null.
        assert_eq!(
            update_body("dev-1", None, None, &TraitChange::OnOff(false))[0][0][0][1],
            json!(["", ""])
        );
    }

    #[test]
    fn a_thermostat_change_is_one_entry_and_always_has_both_fields() {
        let body = update_body(
            "dev-1",
            Some("a"),
            Some("p"),
            &TraitChange::Thermostat {
                mode: "cool".into(),
                setpoint: 23.0,
            },
        );
        let entries = body[0][0][1].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0][0], "temperatureSetting");
        assert_eq!(entries[0][1].as_array().unwrap().len(), 2);
        assert_eq!(entries[0][1][0][0], "mode");
        assert_eq!(entries[0][1][1][0], "thermostatTemperatureSetpoint");
    }

    #[test]
    fn unlock_and_disarm_are_the_sensitive_actions() {
        assert!(TraitChange::Lock(false).is_sensitive());
        assert!(TraitChange::ArmDisarm(false).is_sensitive());
        assert!(!TraitChange::Lock(true).is_sensitive());
        assert!(!TraitChange::ArmDisarm(true).is_sensitive());
        assert!(!TraitChange::OnOff(false).is_sensitive());
    }

    #[test]
    fn support_is_a_suffix_match_on_the_trait_list() {
        let traits = vec![
            "action.devices.traits.OnOff".to_string(),
            "action.devices.traits.Brightness".to_string(),
            "action.devices.traits.ColorSetting".to_string(),
        ];
        assert!(supports(&traits, &TraitChange::OnOff(true)));
        assert!(supports(&traits, &TraitChange::Brightness(1)));
        assert!(supports(&traits, &TraitChange::ColorTemperature(2700)));
        assert!(!supports(&traits, &TraitChange::Volume(1)));
        assert!(!supports(&traits, &TraitChange::Lock(true)));
        assert!(!supports(&[], &TraitChange::OnOff(false)));
    }

    #[test]
    fn the_cli_form_parses_every_supported_trait() {
        assert_eq!(
            TraitChange::parse("onOff", "false", None, None).unwrap(),
            TraitChange::OnOff(false)
        );
        assert_eq!(
            TraitChange::parse("brightness", "40", None, None).unwrap(),
            TraitChange::Brightness(40)
        );
        assert_eq!(
            TraitChange::parse("color.colorTemperature", "2700", None, None).unwrap(),
            TraitChange::ColorTemperature(2700)
        );
        assert_eq!(
            TraitChange::parse("volume.currentVolume", "30", None, None).unwrap(),
            TraitChange::Volume(30)
        );
        assert_eq!(
            TraitChange::parse("lockUnlock.isLocked", "false", None, None).unwrap(),
            TraitChange::Lock(false)
        );
        assert_eq!(
            TraitChange::parse("armDisarm.isArmed", "false", None, None).unwrap(),
            TraitChange::ArmDisarm(false)
        );
        assert_eq!(
            TraitChange::parse("thermostat", "heat", Some("heat"), Some(21.0)).unwrap(),
            TraitChange::Thermostat {
                mode: "heat".into(),
                setpoint: 21.0,
            }
        );
        // <value> can stand in for the setpoint.
        assert_eq!(
            TraitChange::parse("thermostat", "21", Some("heat"), None).unwrap(),
            TraitChange::Thermostat {
                mode: "heat".into(),
                setpoint: 21.0,
            }
        );
        // Case-insensitive trait names, on/off as booleans.
        assert_eq!(
            TraitChange::parse("ONOFF", "on", None, None).unwrap(),
            TraitChange::OnOff(true)
        );
    }

    #[test]
    fn bad_values_and_unknown_traits_are_usage_errors() {
        let code = |r: Result<TraitChange, (home_store::ErrorBody, home_store::Exit)>| {
            r.unwrap_err().1.code()
        };
        assert_eq!(code(TraitChange::parse("onOff", "maybe", None, None)), 2);
        assert_eq!(code(TraitChange::parse("brightness", "101", None, None)), 2);
        assert_eq!(
            code(TraitChange::parse(
                "color.colorTemperature",
                "900",
                None,
                None
            )),
            2
        );
        assert_eq!(code(TraitChange::parse("nonsense", "1", None, None)), 2);
        assert_eq!(
            code(TraitChange::parse("thermostat", "heat", Some("heat"), None)),
            2,
            "a thermostat without a setpoint is usage"
        );
        assert_eq!(
            code(TraitChange::parse("thermostat", "21", None, None)),
            2,
            "a thermostat without a mode is usage"
        );
    }
}
