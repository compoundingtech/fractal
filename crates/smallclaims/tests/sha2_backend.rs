use sha2::{Digest, Sha256, Sha512};

fn enables_asm(dependency: &toml::Value) -> bool {
    dependency
        .get("features")
        .and_then(toml::Value::as_array)
        .is_some_and(|features| features.iter().any(|feature| feature.as_str() == Some("asm")))
}

#[test]
fn sha2_asm_is_declared_only_for_aarch64() {
    // This guards the manifest policy, not Cargo's resolved feature graph.
    let manifest: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    assert!(!enables_asm(&manifest["dependencies"]["sha2"]));
    let targets = manifest["target"].as_table().unwrap();
    let aarch64 = "cfg(target_arch = \"aarch64\")";
    assert!(enables_asm(&targets[aarch64]["dependencies"]["sha2"]));
    for (target, configuration) in targets {
        if target != aarch64
            && let Some(dependency) = configuration
                .get("dependencies")
                .and_then(|dependencies| dependencies.get("sha2"))
        {
            assert!(!enables_asm(dependency), "unexpected asm feature for {target}");
        }
    }
    let workspace: toml::Value = toml::from_str(include_str!("../../../Cargo.toml")).unwrap();
    assert_eq!(workspace["workspace"]["resolver"].as_str(), Some("3"));
}

#[test]
fn sha2_backends_preserve_standard_digest_vectors() {
    assert_eq!(
        hex::encode(Sha256::digest(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex::encode(Sha512::digest(b"abc")),
        concat!(
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a",
            "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        )
    );
}
