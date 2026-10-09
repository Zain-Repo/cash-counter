use cash_counter::updater::{validate_release, verify_file};
use std::fs;

fn release(version: &str) -> serde_json::Value {
    serde_json::json!({"tag_name": format!("v{version}"), "draft": false, "prerelease": false,
        "assets": [{"name": format!("CashLedger-{version}-windows-x64-setup.exe"),
        "browser_download_url": format!("https://github.com/Zain-Repo/cash-counter/releases/download/v{version}/CashLedger-{version}-windows-x64-setup.exe"),
        "digest": format!("sha256:{}", "a".repeat(64)), "size": 123}]})
}

#[test]
fn only_newer_stable_official_release_is_accepted() {
    let valid = release("1.2.3");
    assert_eq!(
        validate_release(&serde_json::to_vec(&valid).unwrap(), "1.0.0")
            .unwrap()
            .unwrap()
            .version,
        "1.2.3"
    );
    assert!(
        validate_release(&serde_json::to_vec(&valid).unwrap(), "1.2.3")
            .unwrap()
            .is_none()
    );
    assert!(
        validate_release(&serde_json::to_vec(&valid).unwrap(), "2.0.0")
            .unwrap()
            .is_none()
    );
    for key in ["draft", "prerelease"] {
        let mut changed = valid.clone();
        changed[key] = true.into();
        assert!(
            validate_release(&serde_json::to_vec(&changed).unwrap(), "1.0.0")
                .unwrap()
                .is_none()
        );
    }
    assert!(
        validate_release(
            &serde_json::to_vec(&release("2.0.0-beta.1")).unwrap(),
            "1.0.0"
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn rejects_redirected_metadata_missing_digest_and_invalid_sizes() {
    for (key, value) in [
        (
            "browser_download_url",
            serde_json::json!("https://example.com/setup.exe"),
        ),
        ("digest", serde_json::Value::Null),
        ("digest", serde_json::json!("sha256:xyz")),
        ("size", serde_json::json!(0)),
        ("size", serde_json::json!(u64::MAX)),
    ] {
        let mut changed = release("1.2.3");
        changed["assets"][0][key] = value;
        assert!(validate_release(&serde_json::to_vec(&changed).unwrap(), "1.0.0").is_err());
    }
}

#[test]
fn checksum_and_size_are_both_required() {
    let directory = tempfile::tempdir().unwrap();
    let installer = directory.path().join("installer.exe");
    fs::write(&installer, b"abc").unwrap();
    let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert!(verify_file(&installer, digest, 3).is_ok());
    assert!(verify_file(&installer, digest, 4).is_err());
    assert!(verify_file(&installer, &"0".repeat(64), 3).is_err());
    fs::write(&installer, b"abd").unwrap();
    assert!(verify_file(&installer, digest, 3).is_err());
}
