//! The part of `store::resource_preparation` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation_shared_seats::{BEFORE_VALIDATE};
use super::*;
pub use fractal_delivery::store::resource_preparation::*;
#[cfg(any(test, feature = "test-support"))]
pub use fractal_store::store::resource_preparation::tests_shared_seats;
pub use fractal_delivery::store::resource_preparation::{Fence, NativeCut, Prepared};
#[cfg(test)]
#[allow(hidden_glob_reexports)] // fractal-store's module of the same name is private; this one holds the tests that moved
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
    /// Record each item's change as `vcs.item.state` alone (see [`crate::model::MirroredPrior`]).
    pub compact: bool,
}

pub struct Observation<'a> {
    pub observer: &'a str,
    pub desired_revision: &'a str,
    pub attempt: Option<&'a str>,
    pub resource: &'a str,
    pub facts: &'a Value,
    pub subscriptions: &'a [(String, SubscriptionSpec)],
    /// The poller's mirror for a GitHub repository, when it has a filled one.
    pub mirrored: Option<&'a crate::model::MirroredPrior>,
}

#[allow(clippy::too_many_arguments)]
pub fn inputs(
    transaction: &Connection,
    observer: &str,
    desired_revision: &str,
    attempt: Option<&str>,
    resource: &str,
    facts: &Value,
    subscriptions: &[(String, SubscriptionSpec)],
    mirrored: Option<&crate::model::MirroredPrior>,
) -> Result<Inputs, St3Error> {
    fractal_store::store::completion_state::flush(transaction).map_err(internal)?;
    read_inputs(transaction, Observation {
        observer, desired_revision, attempt, resource, facts, subscriptions, mirrored,
    })
}

/// Read prepared inputs from the connection's committed snapshot, without mutating that cut.
pub fn read_inputs(
    transaction: &Connection,
    observation: Observation<'_>,
) -> Result<Inputs, St3Error> {
    let Observation { observer, desired_revision, attempt, resource, facts, subscriptions, mirrored } = observation;
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
    let resource_observation =
        fractal_store::store::resources::preparation_observation(transaction, resource)
            .map_err(internal)?;
    let previous = resource_observation.actual.as_ref()
        .and_then(|actual| actual.get("facts").cloned());
    let normalized_observation = fractal_store::store::normalize_resource_observation_with_actuals(
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
        resource_observation.kind,
        || Ok(resource_observation.actual),
    )?
    .expect("a resource observation is normalized");
    let facts = normalized_observation
        .get("facts")
        .expect("a normalized resource observation has facts");
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
        for item in current_object
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(number) = item.get("number").and_then(Value::as_u64) else {
                continue;
            };
            let subject = format!("{resource}/{segment}/{number}");
            let observation =
                fractal_store::store::resources::preparation_observation(transaction, &subject)
                    .map_err(internal)?;
            let graph = observation.actual.and_then(|actual| actual.get("facts").cloned());
            // The graph is exact for every compact fact and every fact a wake or a delivery reads:
            // each change to one is recorded, whatever the mode. A look's prior takes those from
            // the graph, so a mirror that missed a look cannot make a delivered comment new again.
            let exact = |mut base: serde_json::Map<String, Value>, graph: &Value| {
                for name in fractal_schema::github_plugin::COMPACT_FACTS.iter()
                    .chain(fractal_schema::github_plugin::DELIVERY_FACTS)
                {
                    match graph.get(*name) {
                        Some(value) => base.insert((*name).into(), value.clone()),
                        None => base.remove(*name),
                    };
                }
                if let Some(execution) = graph.pointer("/required_checks/execution")
                    && let Some(required) = base.get_mut("required_checks").and_then(Value::as_object_mut)
                {
                    required.insert("execution".into(), execution.clone());
                }
                Value::Object(base)
            };
            let recorded = match (mirrored.and_then(|mirrored| mirrored.items.get(&subject)), graph) {
                // The mirror supplies what compact mode no longer records, such as the title.
                (Some(Value::Object(mirror)), Some(graph)) => Some(exact(mirror.clone(), &graph)),
                (Some(mirror), None) => Some(mirror.clone()),
                // Without the mirror, after compact-only changes, take the rest as this look found
                // it: the facts that drifted are a baseline and deliver nothing.
                (_, Some(graph)) if fractal_store::store::resources::compact_newer(transaction, &subject).map_err(internal)? => {
                    Some(exact(super::item_facts(resource, field, number, Some(&graph), item), &graph))
                }
                (_, graph) => graph,
            };
            let observed_at = observation.observed_at;
            recorded_items.insert(subject, recorded.map(|facts| (facts, observed_at)));
        }
    }
    let mut discovered = Vec::new();
    for (field, _, _) in REPOSITORY_COLLECTIONS {
        let items = discovered_collection_items(
            resource,
            field,
            previous.as_ref(),
            facts,
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
    let observer_health_is_current = fractal_store::store::completion_state::actual(transaction, observer)
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
        let current_status = fractal_store::store::completion_state::actual(transaction, subscription_subject)
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
        compact: mirrored.is_some_and(|mirrored| mirrored.compact),
    })
}
