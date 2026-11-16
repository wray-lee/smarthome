# 07 — Full Web Panel

**What to build:** The Web Panel generates controls from each Device's traits, shows Tuya Telemetry, stays responsive with optimistic updates, polls without triggering rate limits, and guides the user through re-auth for either Backend.

**Blocked by:** 06 — Multi-arch Docker image and minimal Web Panel

**Status:** done

- [x] Controls are generated from traits (on/off, brightness, color temperature, thermostat, volume). An unknown trait shows read-only state rather than a dead control
- [x] Tuya plugs show power, current, voltage, and energy with units
- [x] An action updates the UI immediately, then reconciles from real state read later
- [x] Polling runs only while the page is visible, at most once every 10 seconds
- [x] When Tuya needs re-auth, the panel shows a fresh QR code and recovers after it is scanned
- [x] When Google Home needs re-auth, the panel explains how to sign in again
- [x] Empty, loading, and error states are visible for every data view
- [x] Every control is keyboard reachable with a visible focus style, status is never conveyed by color alone, text contrast meets WCAG AA, and tap targets are at least 44px
- [x] Layout reflows on mobile with no horizontal overflow
