//! Shared mission/step rendering; values in embedded declarations are never inspected or emitted.
use kdl::KdlNode;
use crate::gate_render::{block, node, property, render_gate_node, scalar, text};
use crate::loop_render::comment;
use crate::model::{BaselineSpec, GateSpec, ProductSpec, St3Error, WorkSelector};

pub fn protection(root: &mut KdlNode, human: bool, reviewer: Option<&str>, timeout: Option<u64>) {
    if human { property(root, "revisions", "human-only"); }
    if let Some(reviewer) = reviewer { property(root, "revision-reviewer", reviewer); }
    if let Some(timeout) = timeout { property(root, "timeout", format!("{timeout}ms")); }
}

pub fn annotations(
    root: &mut KdlNode,
    declaration: Option<&str>,
    owners: &[String],
    constraints: &std::collections::BTreeMap<String, Vec<String>>,
) {
    if declaration.is_some() || !owners.is_empty() || !constraints.is_empty() {
        // Debug escapes every line separator, including Unicode separators JSON leaves literal.
        comment(root, &format!("embedded declaration KDL redacted; revision owners: {owners:?}; agent constraints: {constraints:?}"));
    }
}

pub fn common_body(
    goals: &[String],
    constraints: &[String],
    selector: Option<&WorkSelector>,
    baselines: &[BaselineSpec],
    products: &[ProductSpec],
    gates: &[GateSpec],
) -> Result<Vec<KdlNode>, St3Error> {
    let mut children = Vec::new();
    children.extend(goals.iter().map(|goal| text("goal", goal)));
    children.extend(constraints.iter().map(|constraint| text("constraint", constraint)));
    match selector {
        Some(WorkSelector::Assigned { agent }) => children.push(text("assigned-to", agent)),
        Some(WorkSelector::Available { agents }) => children.extend(agents.iter().map(|agent| text("available-to", agent))),
        Some(WorkSelector::Agentless) => children.push(node("agentless", [])),
        None => {}
    }
    for baseline in baselines {
        let predicates = baseline.gates.iter().map(|gate|
            Ok(render_gate_node(gate)?.children().unwrap().nodes()[0].clone()))
            .collect::<Result<_, St3Error>>()?;
        children.push(block(text("baseline", &baseline.name), predicates));
    }
    if !products.is_empty() {
        let products = products.iter().map(|product| {
            let kind = product.subject.split('/').next().ok_or_else(||
                St3Error::new("unrenderable-mission", "product has no subject kind"))?;
            let fields = product.fields.iter().map(|(name, value)| Ok(node(name, [scalar(value)?])))
                .collect::<Result<_, St3Error>>()?;
            Ok(block(text(kind, &product.subject), fields))
        }).collect::<Result<_, St3Error>>()?;
        children.push(block(node("produces", []), products));
    }
    children.extend(gates.iter().map(render_gate_node).collect::<Result<Vec<_>, _>>()?);
    Ok(children)
}
