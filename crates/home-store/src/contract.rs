//! The output and exit-code contract: stdout is always exactly one JSON
//! document, and the exit code says what happened without parsing it.

/// Exit codes from the spec. `Usage` through `Upstream` are failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Ok = 0,
    Usage = 2,
    Ambiguous = 3,
    Offline = 4,
    ReauthRequired = 5,
    Refused = 6,
    Upstream = 7,
}

impl Exit {
    pub fn code(self) -> i32 {
        self as i32
    }
}

/// One error shape for every command: `{"ok":false,"error":{"code","message","…"}}`.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: String,
    pub backend: Option<&'static str>,
    pub candidates: Vec<String>,
}

impl ErrorBody {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        ErrorBody {
            code,
            message: message.into(),
            backend: None,
            candidates: Vec::new(),
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new("usage", message)
    }

    pub fn backend(mut self, backend: &'static str) -> Self {
        self.backend = Some(backend);
        self
    }

    pub fn candidates(mut self, candidates: Vec<String>) -> Self {
        self.candidates = candidates;
        self
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut error = serde_json::Map::new();
        error.insert("code".into(), self.code.into());
        error.insert("message".into(), self.message.clone().into());
        if let Some(backend) = self.backend {
            error.insert("backend".into(), backend.into());
        }
        if !self.candidates.is_empty() {
            error.insert("candidates".into(), self.candidates.clone().into());
        }
        serde_json::json!({"ok": false, "error": error})
    }

    /// The document and the exit code that go with it.
    pub fn exit_with(self, exit: Exit) -> (serde_json::Value, i32) {
        (self.to_json(), exit.code())
    }
}

/// A successful result: `{"ok":true,"data":…}`.
pub fn ok(data: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"ok": true, "data": data})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_their_code_and_optional_fields() {
        let plain = ErrorBody::usage("bad flag").to_json();
        assert_eq!(plain["ok"], false);
        assert_eq!(plain["error"]["code"], "usage");
        assert_eq!(plain["error"]["message"], "bad flag");
        assert!(plain["error"].get("backend").is_none());

        let named = ErrorBody::new("reauth_required", "sign in again")
            .backend("google_home")
            .candidates(vec!["a".into(), "b".into()])
            .to_json();
        assert_eq!(named["error"]["backend"], "google_home");
        assert_eq!(named["error"]["candidates"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn exit_codes_match_the_spec() {
        let codes: Vec<i32> = [
            Exit::Ok,
            Exit::Usage,
            Exit::Ambiguous,
            Exit::Offline,
            Exit::ReauthRequired,
            Exit::Refused,
            Exit::Upstream,
        ]
        .iter()
        .map(|e| e.code())
        .collect();
        assert_eq!(codes, [0, 2, 3, 4, 5, 6, 7]);
    }
}
