//! The panel and the login page each carry their own three-language dictionary.
//!
//! Each language is one flat block between `/* i18n:<id> */` and `/* /i18n */`,
//! one `"key":"value",` per line. The blocks must agree key for key, no value
//! may be empty, and no copy may carry an em dash.

use std::collections::BTreeMap;

const PANEL: &str = include_str!("../src/web/index.html");
const LOGIN: &str = include_str!("../src/web/login.html");

fn entry(line: &str) -> (String, String) {
    let rest = line
        .strip_prefix('"')
        .unwrap_or_else(|| panic!("entry must start with a quote: {line}"));
    let (key, rest) = rest
        .split_once("\":\"")
        .unwrap_or_else(|| panic!("entry must look like \"key\":\"value\",: {line}"));
    let value = rest
        .strip_suffix("\",")
        .unwrap_or_else(|| panic!("entry must end with \",: {line}"));
    assert!(
        !value.contains('"'),
        "value may not contain a quote: {line}"
    );
    assert!(
        !value.contains("/*"),
        "value may not contain a comment opener: {line}"
    );
    (key.to_string(), value.to_string())
}

fn dicts(src: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let mut lang: Option<String> = None;
    let mut cur: BTreeMap<String, String> = BTreeMap::new();
    for line in src.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("/* i18n:") {
            let id = rest
                .strip_suffix(" */")
                .unwrap_or_else(|| panic!("bad i18n marker: {line}"));
            assert!(!id.is_empty(), "empty i18n language id: {line}");
            assert!(lang.is_none(), "nested i18n block: {id}");
            lang = Some(id.to_string());
            cur.clear();
            continue;
        }
        if trimmed == "/* /i18n */" {
            let id = lang.take().expect("/i18n without an opening marker");
            assert!(!out.contains_key(&id), "duplicate i18n block: {id}");
            out.insert(id, std::mem::take(&mut cur));
            continue;
        }
        if lang.is_none() || trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let (key, value) = entry(trimmed);
        assert!(
            cur.insert(key.clone(), value).is_none(),
            "duplicate key in {}: {key}",
            lang.as_deref().unwrap_or("")
        );
    }
    assert!(lang.is_none(), "unclosed i18n block");
    out
}

fn sources() -> [(&'static str, &'static str); 2] {
    [("panel", PANEL), ("login", LOGIN)]
}

#[test]
fn the_three_dictionaries_agree_on_every_key() {
    for (label, src) in sources() {
        let d = dicts(src);
        for id in ["en", "zh-CN", "ja"] {
            assert!(d.contains_key(id), "{label} misses i18n block: {id}");
        }
        let en = &d["en"];
        assert!(!en.is_empty(), "the {label} en block has no entries");
        for id in ["zh-CN", "ja"] {
            let other = &d[id];
            let missing: Vec<&String> = en.keys().filter(|k| !other.contains_key(*k)).collect();
            let extra: Vec<String> = other
                .keys()
                .filter(|k| !en.contains_key(*k))
                .cloned()
                .collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{label} {id} differs from en: missing {missing:?}, extra {extra:?}"
            );
        }
    }
}

#[test]
fn every_value_is_present_and_free_of_em_dash() {
    for (label, src) in sources() {
        for (id, dict) in dicts(src) {
            for (key, value) in dict {
                assert!(
                    !value.trim().is_empty(),
                    "{label} {id} has an empty value: {key}"
                );
                assert!(
                    !value.contains('\u{2014}'),
                    "{label} {id} carries an em dash: {key} = {value}"
                );
            }
        }
    }
}

/// Simplified Chinese uses Chinese quotation marks, not the Japanese corner
/// brackets, and the English copy uses US spelling to match "Color temperature".
#[test]
fn chinese_copy_uses_chinese_quotes_and_english_uses_us_spelling() {
    let d = dicts(PANEL);
    let zh = &d["zh-CN"];
    let corner: Vec<&String> = zh
        .iter()
        .filter(|(_, v)| v.contains('「') || v.contains('」'))
        .map(|(k, _)| k)
        .collect();
    assert!(
        corner.is_empty(),
        "zh-CN carries Japanese corner brackets: {corner:?}"
    );
    let en = &d["en"];
    let british: Vec<&String> = en
        .iter()
        .filter(|(_, v)| v.contains("colour"))
        .map(|(k, _)| k)
        .collect();
    assert!(
        british.is_empty(),
        "en carries the British spelling colour: {british:?}"
    );
}

#[test]
fn the_markup_defaults_to_english_and_selects_from_the_browser() {
    assert!(
        PANEL.contains("<html lang=\"en\">"),
        "the served markup must default to <html lang=\"en\">"
    );
    for needle in [
        "navigator.languages",
        "home_panel_settings",
        "document.documentElement.lang=",
        "function pickLang(",
        "function t(key",
        "data-i18n",
    ] {
        assert!(PANEL.contains(needle), "panel misses {needle:?}");
    }
}

#[test]
fn the_login_page_defaults_to_english_and_picks_a_language() {
    assert!(
        LOGIN.contains("<html lang=\"en\">"),
        "the login markup must default to <html lang=\"en\">"
    );
    for needle in [
        "navigator.languages",
        "home_panel_settings",
        "document.documentElement.lang=",
        "function pickLang(",
        "function t(key",
        "data-i18n",
    ] {
        assert!(LOGIN.contains(needle), "login misses {needle:?}");
    }
}

/// The server renders the alert with a stable `data-code`; the page translates
/// it from this dictionary, so both codes must exist.
#[test]
fn the_login_dictionary_covers_the_server_error_codes() {
    let labels = dicts(LOGIN).remove("en").expect("login en block");
    for key in ["err.wrong_password", "err.rate_limited"] {
        assert!(labels.contains_key(key), "login en misses {key}");
    }
}

#[test]
fn every_static_binding_and_t_call_resolves_to_an_english_key() {
    for (label, src) in sources() {
        let en = dicts(src).remove("en").expect("en block");
        let mut missing = Vec::new();
        for key in attr_keys(src, "data-i18n=\"")
            .into_iter()
            .chain(attr_keys(src, "data-i18n-aria=\""))
            .chain(t_keys(src))
        {
            if !en.contains_key(&key) {
                missing.push(key);
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "{label} keys with no English entry: {missing:?}"
        );
    }
}

fn attr_keys(src: &'static str, prefix: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find(prefix) {
        rest = &rest[i + prefix.len()..];
        let end = rest.find('"').expect("unterminated i18n attribute");
        keys.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    keys
}

fn t_keys(src: &'static str) -> Vec<String> {
    let bytes = src.as_bytes();
    let mut keys = Vec::new();
    for i in 0..bytes.len().saturating_sub(3) {
        if &bytes[i..i + 3] != b"t(\"" {
            continue;
        }
        // Skip matches inside a longer name, e.g. split(".
        if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
            continue;
        }
        let start = i + 3;
        let end = start + src[start..].find('"').expect("unterminated t() key");
        keys.push(src[start..end].to_string());
    }
    keys
}
