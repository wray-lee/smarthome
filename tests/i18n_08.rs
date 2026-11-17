//! The panel carries its own three-language dictionary.
//!
//! Each language is one flat block between `/* i18n:<id> */` and `/* /i18n */`,
//! one `"key":"value",` per line. The blocks must agree key for key, no value
//! may be empty, and no copy may carry an em dash.

use std::collections::BTreeMap;

const PANEL: &str = include_str!("../src/web/index.html");

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

fn dicts() -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let mut lang: Option<String> = None;
    let mut cur: BTreeMap<String, String> = BTreeMap::new();
    for line in PANEL.lines() {
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

#[test]
fn the_three_dictionaries_agree_on_every_key() {
    let d = dicts();
    for id in ["en", "zh-CN", "ja"] {
        assert!(d.contains_key(id), "missing i18n block: {id}");
    }
    let en = &d["en"];
    assert!(!en.is_empty(), "the en block has no entries");
    for id in ["zh-CN", "ja"] {
        let other = &d[id];
        let missing: Vec<&String> = en.keys().filter(|k| !other.contains_key(*k)).collect();
        let extra: Vec<&String> = other.keys().filter(|k| !en.contains_key(*k)).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "{id} differs from en: missing {missing:?}, extra {extra:?}"
        );
    }
}

#[test]
fn every_value_is_present_and_free_of_em_dash() {
    for (id, dict) in dicts() {
        for (key, value) in dict {
            assert!(!value.trim().is_empty(), "{id} has an empty value: {key}");
            assert!(
                !value.contains('\u{2014}'),
                "{id} carries an em dash: {key} = {value}"
            );
        }
    }
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
fn every_static_binding_and_t_call_resolves_to_an_english_key() {
    let en = dicts().remove("en").expect("en block");
    let mut missing = Vec::new();
    for key in attr_keys("data-i18n=\"")
        .into_iter()
        .chain(attr_keys("data-i18n-aria=\""))
        .chain(t_keys())
    {
        if !en.contains_key(&key) {
            missing.push(key);
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "keys with no English entry: {missing:?}"
    );
}

fn attr_keys(prefix: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = PANEL;
    while let Some(i) = rest.find(prefix) {
        rest = &rest[i + prefix.len()..];
        let end = rest.find('"').expect("unterminated i18n attribute");
        keys.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    keys
}

fn t_keys() -> Vec<String> {
    let bytes = PANEL.as_bytes();
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
        let end = start + PANEL[start..].find('"').expect("unterminated t() key");
        keys.push(PANEL[start..end].to_string());
    }
    keys
}
