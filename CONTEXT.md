# Smarthome

A personal tool for agents and humans to control smart home devices. It reaches devices through two independent Backends.

## Language

**Backend**:
An independent channel the tool uses to reach Devices. There are two: **Google Home** and **Tuya**. Neither depends on the other.

**Google Home**:
The Backend for devices from any vendor. It can operate devices (for example on/off) but exposes no detailed telemetry such as energy use.

**Tuya**:
The Backend that talks to Tuya devices directly, used to get what Google Home cannot show, such as power, current, voltage, and energy.

**Device**:
Something that can be controlled or read, as seen through one Backend. One physical plug may appear as a Device in both Backends.

**Local Key**:
The per-device secret that allows Local Control of a Tuya Device. Once the tool holds it, the Device can be operated without the Tuya cloud, until the Device is reset or re-paired.

**Local Control**:
Talking to a Device directly over the home network, with no vendor cloud involved. Deferred; the Tuya Backend currently uses Cloud Control.
_Avoid_: LAN mode, offline mode

**Cloud Control**:
Operating a Tuya Device through the Smart Life cloud, signed in as the user's Smart Life account. Works from any network.

**Vendor Exit**:
Keeping Devices usable if a vendor's cloud disappears. For Tuya, the first step is holding every Local Key. The last resort is reflashing a Device with third-party firmware, which turns it into a Device of a different Backend.

**Telemetry**:
Measured readings from a Device, such as power, current, voltage, and accumulated energy. Only the Tuya Backend provides it.
_Avoid_: stats, details

**Linked Service**:
A vendor cloud account connected into Google Home (for example Smart Life for Tuya devices). The Google Home Backend never talks to a Linked Service directly.
_Avoid_: integration, provider

**Sensitive Action**:
An action that weakens physical security, such as unlocking a door or disarming an alarm. The tool refuses it unless the caller confirms explicitly.

**Protected Device**:
A Device the tool refuses to write to, through any Backend, until the user removes the protection with `home protect --remove`. Reading its state and Telemetry still works. Protection stays active across writes; two explicit one-time overrides each let exactly one write through without changing the list: `--force` on the CLI, and re-entering the panel password in the Web Panel for that one request. Both exist for the human at the keyboard, and the same failed-password backoff covers the panel override as the login. Unlike a Sensitive Action, which is about the action, protection is about the Device, and a Protected Device still needs `--confirm` for a Sensitive Action.
_Avoid_: locked device (confused with door locks), child lock (a physical-button setting on the plug)

**CLI**:
The command-line interface, designed to be called by agents. Its output is machine-readable.

**Web Panel**:
A browser UI, served from the Docker deployment, for humans to control Devices.

## Relationships

- A **Linked Service** contributes zero or more **Devices** to **Google Home**
- A physical Tuya plug can be one **Device** in **Google Home** (via the Smart Life **Linked Service**) and another **Device** in the **Tuya** Backend
- **Telemetry** comes only from the **Tuya** Backend

## Example dialogue

> **Dev:** "The agent wants today's energy use for the desk plug. Which Backend?"
> **Domain expert:** "**Tuya**. **Google Home** only knows the plug is on."

## Flagged ambiguities

- "Adapt for Google Home" was used to mean both "expose Tuya devices to Google Home" and "control Google Home from a CLI". Resolved: the tool controls Google Home as a Backend.
- The Tuya work was first dropped as "just link it into Google Home", then restored because Google Home has no Telemetry. Resolved: two independent Backends.
