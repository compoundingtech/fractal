//! The part of `store::resource_preparation` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation_shared_seats::{BEFORE_VALIDATE};
pub use super::*;
pub use st3_store::store::resource_preparation::*;
#[cfg(any(test, feature = "test-support"))]
pub use st3_store::store::resource_preparation::tests_shared_seats;
pub use st3_store::store::resource_preparation::{Fence, NativeCut, Prepared};
#[cfg(test)]
#[allow(hidden_glob_reexports)] // st3-store's module of the same name is private; this one holds the tests that moved
mod tests;

pub struct Inputs {
    pub active_subscriptions: Vec<(String, SubscriptionSpec)>,
    pub normalized_observation: BTreeMap<String, Value>,
    pub previous: Option<Value>,
    pub baseline: bool,
    pub repository_facts: Value,
    pub legacy_collections: bool,
    pub changed_fields: Vec<String>,
    pub repository_changed_fields: Vec<String>,
    pub recorded_items: HashMap<String, Option<(Value, u128)>>,
    pub discovered: Vec<(&'static str, DiscoveredItem)>,
    pub observer_health_is_current: bool,
    pub subscription_states: BTreeMap<String, (bool, &'static str, bool)>,
    pub changed: bool,
    pub should_record: bool,
}

pub struct Observation<'a> {
    pub observer: &'a str,
    pub desired_revision: &'a str,
    pub attempt: Option<&'a str>,
    pub resource: &'a str,
    pub facts: &'a Value,
    pub subscriptions: &'a [(String, SubscriptionSpec)],
}

pub fn inputs(
    transaction: &Connection,
    observer: &str,
    desired_revision: &str,
    attempt: Option<&str>,
    resource: &str,
    facts: &Value,
    subscriptions: &[(String, SubscriptionSpec)],
) -> Result<Inputs, St3Error> {
    inputs_with_actuals(transaction, Observation {
        observer, desired_revision, attempt, resource, facts, subscriptions,
    }, &mut |subject| latest_actual(transaction, subject))
}

pub fn inputs_with_actuals(
    transaction: &Connection,
    observation: Observation<'_>,
    actual: &mut dyn FnMut(&str) -> Result<Option<Value>>,
) -> Result<Inputs, St3Error> {
    let Observation { observer, desired_revision, attempt, resource, facts, subscriptions } = observation;
    let mut active_subscriptions = Vec::new();
    for (subject, expected) in subscriptions {
        let Some(desired) = current_desired_row_tx(transaction, subject).map_err(internal)? else {
            continue;
        };
        if desired.kind != "subscription" {
            continue;
        }
        let value = serde_json::from_str(&desired.body).map_err(internal)?;
        if crate::graph::subscription_spec(&value).as_ref() == Some(expected) && !expected.stopped {
            active_subscriptions.push((subject.clone(), expected.clone()));
        }
    }
    let normalized_observation = normalize_resource_observation(
        transaction,
        &ClaimInput {
            subject: resource.into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: BTreeMap::from([("facts".into(), facts.clone())]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        },
    )?
    .expect("a resource observation is normalized");
    let facts = normalized_observation
        .get("facts")
        .expect("a normalized resource observation has facts");
    let previous = actual(resource)
        .map_err(internal)?
        .and_then(|actual| actual.get("facts").cloned());
    let baseline = previous.is_none();
    let previous_object = previous.as_ref().and_then(Value::as_object);
    let current_object = facts.as_object().ok_or_else(|| {
        St3Error::new(
            "invalid-resource-observation",
            "resource observation facts must be an object",
        )
    })?;
    // A repository's collections name the items an observation saw. Each item is its
    // own resource, so the repository resource keeps only its own facts and a change
    // to one item records only that item.
    let collection = |field: &str| {
        REPOSITORY_COLLECTIONS
            .iter()
            .any(|(name, _, _)| *name == field)
    };
    let repository_facts = Value::Object(
        current_object
            .iter()
            .filter(|(field, _)| !collection(field))
            .map(|(field, value)| (field.clone(), value.clone()))
            .collect(),
    );
    let legacy_collections =
        previous_object.is_some_and(|previous| previous.keys().any(|field| collection(field)));
    let mut changed_fields = BTreeSet::new();
    for field in current_object
        .keys()
        .chain(previous_object.into_iter().flat_map(serde_json::Map::keys))
        .filter(|field| !collection(field))
    {
        if previous_object.and_then(|object| object.get(field)) != current_object.get(field) {
            changed_fields.insert(field.clone());
        }
    }
    let repository_changed_fields = changed_fields.iter().cloned().collect::<Vec<_>>();
    let mut recorded_items = HashMap::new();
    for (field, segment, _) in REPOSITORY_COLLECTIONS {
        for number in current_object
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("number").and_then(Value::as_u64))
        {
            let subject = format!("{resource}/{segment}/{number}");
            let recorded = actual(&subject)
                .map_err(internal)?
                .and_then(|actual| actual.get("facts").cloned());
            let observed_at = transaction
                .query_row(
                    /* sqlite-large-value: allow-for-now lv-hcf68b5e0877e owner=st3_missions::store::resource_preparation TODO=trace-statement-and-value-representation */ &canonical_sql(
                        "SELECT accepted_at_unix_ms FROM claims
                                     WHERE subject=?1 AND kind='resource.observed'
                                     ORDER BY COALESCE(json_extract(body, '$.fields.attribution_only'), 0), CANONICAL_DESC(claims) LIMIT 1",
                    ),
                    [&subject],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(internal)?
                .and_then(|at| at.parse::<u128>().ok())
                .unwrap_or_default();
            recorded_items.insert(subject, recorded.map(|facts| (facts, observed_at)));
        }
    }
    let mut discovered = Vec::new();
    for (field, _, _) in REPOSITORY_COLLECTIONS {
        let items = discovered_collection_items(
            resource,
            field,
            previous.as_ref(),
            &facts,
            &mut |subject| {
                recorded_items
                    .get(subject)
                    .cloned()
                    .flatten()
                    .map(|(facts, _)| facts)
            },
        );
        changed_fields.extend(
            items
                .iter()
                .flat_map(|item| item.data_types.iter().cloned()),
        );
        discovered.extend(items.into_iter().map(|item| (field, item)));
    }
    let changed_fields = changed_fields.into_iter().collect::<Vec<_>>();
    let observer_health_is_current = actual(observer)
        .map_err(internal)?
        .is_some_and(|actual| {
            actual.get("state").and_then(Value::as_str) == Some("healthy")
                && actual.get("revision").and_then(Value::as_str) == Some(desired_revision)
                && actual.get("reason").is_none()
        });
    let mut subscription_states = BTreeMap::new();
    for (subscription_subject, subscription) in &active_subscriptions {
        let target_exists = if subscription.delivery == "message" {
            transaction
                .query_row(
                    "SELECT 1 FROM desired WHERE subject=?1 AND kind='agent'",
                    [&subscription.to],
                    |_| Ok(()),
                )
                .optional()
                .map_err(internal)?
                .is_some()
        } else {
            true
        };
        let status = if target_exists { "active" } else { "pending" };
        let current_status = actual(subscription_subject)
            .map_err(internal)?
            .and_then(|actual| {
                actual
                    .get("state")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        subscription_states.insert(
            subscription_subject.clone(),
            (
                target_exists,
                status,
                current_status.as_deref() != Some(status),
            ),
        );
    }
    let changed = baseline || !changed_fields.is_empty() || !discovered.is_empty();
    let should_record = changed
        || legacy_collections
        || attempt.is_some()
        || !observer_health_is_current
        || subscription_states.values().any(|(_, _, changed)| *changed);
    Ok(Inputs {
        active_subscriptions,
        normalized_observation,
        previous,
        baseline,
        repository_facts,
        legacy_collections,
        changed_fields,
        repository_changed_fields,
        recorded_items,
        discovered,
        observer_health_is_current,
        subscription_states,
        changed,
        should_record,
    })
}
