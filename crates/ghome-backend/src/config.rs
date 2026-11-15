//! Every upstream base URL, overridable so tests can point the Backend at a
//! fake. The values come from the environment; nothing else configures the
//! Backend yet.

pub const FLOYER_URL: &str =
    "https://googlehomefoyer-pa.clients6.google.com/$rpc/google.internal.home.foyer.v1.";
pub const GPSOAUTH_URL: &str = "https://android.clients.google.com/auth";

#[derive(Debug, Clone)]
pub struct Config {
    /// Prefix that `StructuresService/GetHomeGraph` is appended to.
    pub foyer_url: String,
    /// The Android account-token endpoint, also used for the oauth exchange.
    pub gpsoauth_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        let get = |key: &str, default: &str| {
            std::env::var(key)
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        Config {
            foyer_url: get("HOME_FOYER_URL", FLOYER_URL),
            gpsoauth_url: get("HOME_GPSOAUTH_URL", GPSOAUTH_URL),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config::from_env()
    }
}
