use super::*;

fn fact(id: &str, identity: &str, from: &str, to: &str, previous: Option<&str>) -> Fact {
    Fact { id: id.into(), identity: identity.into(), from: from.into(), to: to.into(), previous: previous.map(str::to_owned), repaired: false, unavailable: None }
}
fn project(facts: Vec<Fact>, declarations: &[&str], proving: &str) -> Projection<()> {
    let mut heads = BTreeMap::new();
    let mut names = BTreeMap::new();
    for identity in declarations {
        heads.insert((*identity).into(), Head { kind: (), canonical: (*identity).into(), revision: None, blocked: None });
        names.insert((*identity).into(), ((*identity).into(), None));
    }
    reduce(heads, names, facts, Some(proving))
}

#[test]
fn sibling_branches_conflict_independent_of_input_order() {
    for reverse in [false, true] {
        let mut facts = vec![fact("a", "storage", "storage", "left", None), fact("b", "storage", "storage", "right", None)];
        if reverse { facts.reverse(); }
        let result = project(facts, &["storage"], "a");
        assert!(!result.accepted);
        assert_eq!(result.heads["storage"].blocked, Some("conflict"));
    }
}

#[test]
fn storage_reservations_precede_claim_id_target_winners() {
    let result = project(vec![fact("a", "storage", "storage", "reserved", None)], &["storage", "reserved"], "a");
    assert!(!result.accepted);
    assert_eq!(result.names["reserved"], ("reserved".into(), None));
}

#[test]
fn smallest_claim_wins_and_losing_descendants_stay_fenced() {
    let result = project(vec![
        fact("z", "other", "other", "target", None),
        fact("b", "other", "target", "child", Some("z")),
        fact("a", "storage", "storage", "target", None),
    ], &["storage", "other"], "b");
    assert!(!result.accepted);
    assert_eq!(result.heads["other"].blocked, Some("conflict"));
    assert_eq!(result.heads["storage"].blocked, None);
    assert_eq!(result.names["target"], ("storage".into(), Some("a".into())));
}

#[test]
fn historical_ancestor_is_accepted_after_descendant() {
    let result = project(vec![
        fact("a", "storage", "storage", "renamed", None),
        fact("b", "storage", "renamed", "descendant", Some("a")),
    ], &["storage"], "a");
    assert!(result.accepted);
    assert_eq!(result.heads["storage"].canonical, "descendant");
    assert_eq!(result.heads["storage"].blocked, None);
}

#[test]
fn repaired_sibling_is_not_a_branch_but_reserves_its_target() {
    let mut repaired = fact("a", "storage", "storage", "abandoned", None);
    repaired.repaired = true;
    let result = project(vec![repaired, fact("b", "storage", "storage", "renamed", None)], &["storage"], "b");
    assert!(result.accepted);
    assert_eq!(result.names["abandoned"], ("storage".into(), Some("a".into())));
}

#[test]
fn missing_predecessor_reserves_prior_name_and_holds_identity() {
    let result = project(vec![fact("b", "storage", "prior", "target", Some("a"))], &["storage"], "b");
    assert!(!result.accepted);
    assert_eq!(result.heads["storage"].blocked, Some("pending-replication"));
    assert_eq!(result.names["prior"], ("storage".into(), Some("b".into())));
}
