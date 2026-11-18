use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use fractal_client::{ClientResultBinding, client_result_canonical_bytes, visible_state_canonical_json, visible_state_digest};
use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};

#[test]
fn all_clients_share_canonical_bytes_and_p256_signatures() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../fixtures/clients/client-control-v1.golden.json")).unwrap();
    let key = URL_SAFE_NO_PAD.decode(fixture["public_key_sec1_base64url"].as_str().unwrap()).unwrap();
    let verifier = UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &key);
    for vector in fixture["vectors"].as_array().unwrap() {
        let binding: ClientResultBinding = serde_json::from_value(vector["input"].clone()).unwrap();
        assert_eq!(visible_state_canonical_json(&binding.visible_state).unwrap(), vector["canonical_visible_state"].as_str().unwrap());
        assert_eq!(visible_state_digest(&binding.visible_state).unwrap(), vector["visible_state_sha256"].as_str().unwrap());
        let canonical = client_result_canonical_bytes(&binding).unwrap();
        assert_eq!(canonical, vector["canonical_bytes"].as_str().unwrap().as_bytes());
        let signature = URL_SAFE_NO_PAD.decode(vector["signature_p1363_base64url"].as_str().unwrap()).unwrap();
        verifier.verify(&canonical, &signature).unwrap();
        for negative in vector["negative_bindings"].as_array().unwrap() {
            let changed: ClientResultBinding = serde_json::from_value(negative.clone()).unwrap();
            assert!(verifier.verify(&client_result_canonical_bytes(&changed).unwrap(), &signature).is_err());
        }
        // message.send's ordinary fields-v1 domain cannot consume a result signature.
        let message = fixture["message_domain_bytes"].as_str().unwrap().as_bytes();
        assert!(verifier.verify(message, &signature).is_err());
        // The exact same enrolled test key signed this message-domain vector.
        let message_signature = URL_SAFE_NO_PAD.decode(fixture["message_domain_signature_p1363_base64url"].as_str().unwrap()).unwrap();
        verifier.verify(message, &message_signature).unwrap();
        assert!(verifier.verify(&canonical, &message_signature).is_err());
    }
}
