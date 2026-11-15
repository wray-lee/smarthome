# Google Home is reached through the unofficial foyer API

We control Google Home through foyer, the internal API that the Google Home app and home.google.com use. We authenticate with a master token obtained from a one-time browser login. Google's only official server-side route, the Home MCP server, requires a Google Home Premium Advanced subscription and US English early-access approval, which we don't have. The Assistant SDK can only send natural-language commands and cannot read state, so it was rejected.

## Consequences

- Foyer is undocumented, sits outside Google's API terms, and its fields have changed before (2024, 2026-10). It can break without notice. If it breaks, we capture traffic from the web app ourselves.
- The master token is equivalent to the account password and stays valid until the password changes.
- The Google Home Backend sits behind one narrow interface, so it can be swapped for the Home MCP server if that becomes available to us.
- The starting point is a fork of `piekstra/google-home-cli` (MIT, Rust), with token storage moved to SQLite.
