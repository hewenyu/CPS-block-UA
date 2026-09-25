use cps_block_ua::{DEFAULT_PATTERN, MANIFEST, Mode, Policy};
use gateway_plugin_sdk::{Manifest, call::middleware::MiddlewareHeader};
use serde_json::{Value, json};

fn header(value: &[u8]) -> MiddlewareHeader {
    MiddlewareHeader {
        name: "User-Agent".into(),
        value: value.into(),
    }
}

fn policy(value: Value) -> Policy {
    Policy::from_value(value).unwrap()
}

#[test]
fn defaults_match_author_schema_and_manifest_is_valid() {
    Manifest::from_author_slice(MANIFEST).unwrap();
    let manifest: Value = serde_json::from_slice(MANIFEST).unwrap();
    let properties = &manifest["configurationSchema"]["properties"];
    assert_eq!(properties["mode"]["default"], "observe");
    assert_eq!(properties["allow_models"]["default"], true);
    assert_eq!(
        properties["allow_patterns"]["default"],
        json!([DEFAULT_PATTERN])
    );
    assert_eq!(policy(json!({})).mode(), Mode::Observe);
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn accepts_codex_product_versions_with_realistic_platform_suffixes() {
    let p = policy(json!({"mode":"enforce"}));
    for ua in [
        "codex_cli_rs/0.39.0 (Linux; x86_64)",
        "codex_cli_rs/0.144.0 (linux 6.8; x86_64) xterm (codex_cli_rs; 1.0.0)",
        "Codex Desktop/0.147.0-alpha.6.6 (Mac OS 15.7.1; arm64) unknown (Codex Desktop; 26.803.81509)",
        "codex_vscode/0.114.0 (Windows 11; x86_64) vscode/1.100.0",
        "codex-cli/0.144.0",
        "codex-tui/0.144.0 (Mac OS 15.7; arm64)",
        "codex_exec/0.144.0 (Linux; x86_64)",
        "\t codex_cli_rs/1.0.0 \t",
    ] {
        assert_eq!(
            p.rejection_reason("/v1/responses", &[header(ua.as_bytes())]),
            None,
            "{ua}"
        );
    }
}

#[test]
fn rejects_unrelated_clients_and_brand_substrings() {
    let p = policy(json!({}));
    for ua in [
        "Go-http-client/1.1",
        "curl/8.0",
        "Mozilla/5.0",
        "python-httpx/0.28.1",
        "fake-codex_cli_rs/1.2.3",
        "curl/8.0 codex_cli_rs/1.2.3",
        "codex_cli_rs/1.2.3-malicious/other",
        "codex_cli_rs/1.2.3evil",
        "codex_cli_rs/not-a-version",
        "codex_cli_rs/",
        "CODEX_CLI_RS/1.2.3",
        "codex_cli_rs/1.2.3,curl/8.0",
        "my-Codex Desktop/1.2.3",
    ] {
        assert_eq!(
            p.rejection_reason("/v1/responses", &[header(ua.as_bytes())]),
            Some("unmatched_ua"),
            "{ua}"
        );
    }
}

#[test]
fn rejects_missing_empty_duplicate_and_malformed_values() {
    let p = policy(json!({}));
    assert_eq!(p.rejection_reason("/v1/responses", &[]), Some("missing_ua"));
    for value in [b"".as_slice(), b" \t "] {
        assert_eq!(
            p.rejection_reason("/v1/responses", &[header(value)]),
            Some("empty_ua")
        );
    }
    for value in [
        b"\xff".as_slice(),
        b"codex_cli_rs/1.2.3\r\nX: y",
        b"codex_cli_rs/1.2.3\0",
        b"codex_cli_rs/1.2.3\tother",
        "codex_cli_rs/1.2.3 \u{a0}".as_bytes(),
    ] {
        assert_eq!(
            p.rejection_reason("/v1/responses", &[header(value)]),
            Some("invalid_ua")
        );
    }
    let duplicate = [
        header(b"codex_cli_rs/1.2.3"),
        MiddlewareHeader {
            name: "uSeR-aGeNt".into(),
            value: b"curl/8.0".to_vec(),
        },
    ];
    assert_eq!(
        p.rejection_reason("/v1/responses", &duplicate),
        Some("duplicate_ua")
    );
    assert_eq!(
        p.rejection_reason("/v1/responses", &[header(&vec![b'a'; 4097])]),
        Some("oversized_ua")
    );
}

#[test]
fn models_exemption_is_explicit_and_narrow() {
    let p = policy(json!({"mode":"enforce"}));
    for path in ["/v1/models", "/v1/models/gpt-5.4"] {
        assert_eq!(p.rejection_reason(path, &[]), None);
        assert_eq!(
            policy(json!({"allow_models":false})).rejection_reason(path, &[]),
            Some("missing_ua")
        );
    }
    for path in [
        "/v1/responses",
        "/v1/responses/compact",
        "/v1/models/",
        "/v1/models/x/action",
        "/v1/models/..",
        "/v1/models/%2e%2e",
        "/v1/models?query=1",
        "/v1/models-other",
    ] {
        assert_eq!(p.rejection_reason(path, &[]), Some("missing_ua"), "{path}");
    }
}

#[test]
fn configured_patterns_replace_defaults_and_match_the_whole_value() {
    let p = policy(json!({"allow_patterns":["MyClient/[0-9.]+", "Other/1"]}));
    for ua in ["MyClient/1.2", "Other/1"] {
        assert_eq!(
            p.rejection_reason("/v1/responses", &[header(ua.as_bytes())]),
            None
        );
    }
    for ua in [
        "codex_cli_rs/1.2.3",
        "prefix MyClient/1.2",
        "Other/1 suffix",
        "MyClient/1.2\n",
    ] {
        assert!(
            p.rejection_reason("/v1/responses", &[header(ua.as_bytes())])
                .is_some()
        );
    }
}

#[test]
fn invalid_configuration_never_silently_disables_the_policy() {
    for value in [
        Value::Null,
        json!([]),
        json!({"mode":"typo"}),
        json!({"mod":"enforce"}),
        json!({"allow_models":"true"}),
        json!({"allow_patterns":[]}),
        json!({"allow_patterns":[""]}),
        json!({"allow_patterns":["["]}),
        json!({"allow_patterns":["foo)|(?:bar"]}),
        json!({"allow_patterns":["a".repeat(1025)]}),
        json!({"allow_patterns":vec!["a";33]}),
    ] {
        assert!(Policy::from_value(value).is_err());
    }
}
