# Protected Devices take a one-time override, and password checks share backoff

Protection is a property of the Device: a write to a Protected Device is
refused with exit 6 (`protected`). The human sometimes needs one write through
it without giving up the protection that keeps every other write out. We allow
exactly two overrides, each covering one call: `--force` on the CLI, and
re-entering the panel password on the Web Panel. Neither edits the protected
list; `home protect --remove` remains the only way to end protection.

Both overrides live in `home_store::guard`, in `check_with` / `require_with`.
That module is already the single gate every write path asks - both Backends,
CLI and panel - so putting the override there means one implementation instead
of one per caller, and the default (`check` / `require`) still refuses. A
forced write returns `Allowed` and never touches `protected_devices`, so the
list is identical before and after.

`--force` stays separate from `--confirm`. They refuse different things:
`--confirm` accepts a Sensitive Action (unlocking a lock, disarming an alarm),
`--force` accepts a write to a Protected Device. Collapsing them would let one
flag defeat both guards. The order is fixed: the guard runs first, the
Sensitive Action check second, so a protected lock needs `--force` *and*
`--confirm`, and `--force` alone still exits 6 (`sensitive_action`) with zero
upstream calls.

On the Web Panel the override is the panel password in the POST body, present
for that request only. It is never stored, logged, or echoed back. This is a
second place a password arrives from the network, so it shares one
`AuthThrottle` with `POST /login`: a single failure counter and block window
behind a mutex. A blocked request gets `429 Too Many Requests` with
`Retry-After` *before* any password comparison, so a right password during the
backoff neither succeeds nor reveals that it was right. A match resets the
counter; each mismatch adds a failure, and from the third the block grows
exponentially (`2 << min(failures - 3, 5)` seconds). Splitting the throttles
would let an attacker reset one by switching endpoints.

## Consequences

- The guard keeps one refusal path; the override is a parameter, not a bypass
  flag scattered through the Backends.
- `--force` and the panel password are human-only escape hatches. Agents never
  use them; `AGENTS.md` states that.
- Anyone holding the panel password can write a protected Device, one call at
  a time. The throttle slows guessing; it is not a lockout, so the panel still
  belongs on loopback behind a private network.
- A supplied-but-wrong password on an *unprotected* Device also returns 403
  `protected`, because any supplied password is verified.
- `home_store::guard` now owns both the policy and its override; a future
  third override belongs there too, not in a caller.
