use fractal_client::{CollectionEvent, Resource};

#[test]
fn commands_collection_decodes_typed_grant_lifecycle_vectors() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!("../../../fixtures/clients/client-control-lifecycle-v1.json")).unwrap();
    for row in vectors["resources"].as_array().unwrap() {
        let resource: Resource = serde_json::from_value(row.clone()).unwrap();
        let Resource::ClientControlGrantLifecycle(lifecycle) = resource else { panic!("lifecycle must never decode as Unknown") };
        assert_eq!(lifecycle.grant_id, row["grant_id"].as_str().unwrap());
        assert_eq!(lifecycle.item_id.as_deref(), row["item_id"].as_str());
        assert_eq!(lifecycle.sequence, row["sequence"].as_u64().unwrap());
        assert_eq!(serde_json::to_value(lifecycle.lifecycle.state).unwrap(), row["state"]);
    }
    for frame in vectors["frames"].as_array().unwrap() {
        let event: CollectionEvent = serde_json::from_value(frame.clone()).unwrap();
        let (collection, rows) = match event {
            CollectionEvent::Snapshot { collection, items, .. } => (collection, items),
            CollectionEvent::Changes { collection, upserts, .. } => (collection, upserts),
            _ => panic!("lifecycle uses existing snapshot/changes frames only"),
        };
        assert_eq!(collection, "commands");
        assert!(matches!(&rows[0], Resource::ClientControlGrantLifecycle(_)));
    }
    let mut invalid = vectors["resources"][0].clone();
    invalid["state"] = "future-state".into();
    assert!(serde_json::from_value::<Resource>(invalid).is_err());
}
