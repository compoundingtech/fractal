//! Strict readers of canonical KDL; acquisition configuration and policy share normal desired state.
use serde_json::Value;
use super::{Roster, RosterMember, Source, SourceDefinition, valid_source};

fn children<'a>(node: &'a Value, kind: &str) -> Result<&'a [Value], String> {
    if node["name"] != kind || node.get("properties").and_then(Value::as_object).is_some_and(|properties| !properties.is_empty()) {
        return Err(format!("expected a property-free {kind} declaration"));
    }
    node.get("children").and_then(Value::as_array).map(Vec::as_slice).ok_or_else(|| format!("{kind} requires a body"))
}

fn argument(node: &Value) -> Result<&Value, String> {
    match node.get("arguments").and_then(Value::as_array).map(Vec::as_slice) {
        Some([value]) => Ok(value),
        _ => Err("declaration child requires exactly one argument".into()),
    }
}

fn name(node: &Value) -> Result<&str, String> {
    argument(node)?.as_str().filter(|name| !name.is_empty()).ok_or_else(|| "declaration requires one nonempty name".into())
}

pub fn parse_source(node: &Value) -> Result<SourceDefinition, String> {
    let id = name(node)?;
    let mut scalar = std::collections::BTreeMap::new();
    let mut repositories = std::collections::BTreeSet::new();
    for child in children(node, "github-source")? {
        let child_name = child["name"].as_str().ok_or("source child has no name")?;
        if child.get("children").is_some() || child.get("properties").and_then(Value::as_object).is_some_and(|properties| !properties.is_empty()) {
            return Err(format!("source child {child_name} takes no properties or body"));
        }
        let value = argument(child)?;
        if child_name == "repository" {
            let locator = value.as_str().ok_or("repository locator must be a string")?;
            let mut parts = locator.split('/');
            if parts.next().is_none_or(str::is_empty) || parts.next().is_none_or(str::is_empty) || parts.next().is_some() || !repositories.insert(locator.to_owned()) {
                return Err("repository locators must be unique owner/name pairs".into());
            }
        } else if matches!(child_name, "host" | "generation" | "authority-grant" | "enabled" | "checkpoint-every-ms" | "replicate-private") {
            if scalar.insert(child_name, value).is_some() { return Err(format!("source repeats {child_name}")); }
        } else { return Err(format!("unknown source child {child_name}")); }
    }
    let text = |key: &str| scalar.get(key).and_then(|value| value.as_str()).filter(|value| !value.is_empty()).map(str::to_owned).ok_or_else(|| format!("source needs {key} string"));
    let number = |key: &str| scalar.get(key).and_then(|value| value.as_u64()).ok_or_else(|| format!("source needs {key} integer"));
    let boolean = |key: &str| scalar.get(key).and_then(|value| value.as_bool()).ok_or_else(|| format!("source needs explicit {key} boolean"));
    let enabled = boolean("enabled")?;
    let authority_grant = scalar.get("authority-grant").map(|_| text("authority-grant")).transpose()?;
    if enabled && authority_grant.is_none() {
        return Err("enabled github-source requires an authority-grant".into());
    }
    let definition = SourceDefinition {
        source: Source { host: text("host")?, id: id.to_owned(), generation: number("generation")? },
        authority_grant,
        repositories: repositories.into_iter().collect(),
        enabled,
        checkpoint_interval_ms: number("checkpoint-every-ms")?,
        replicate_private: boolean("replicate-private")?,
    };
    if !valid_source(&definition.source) || definition.repositories.is_empty() || definition.checkpoint_interval_ms < 60_000 {
        return Err("source needs valid identity, repositories and checkpoint cadence of at least one minute".into());
    }
    Ok(definition)
}

pub fn parse_roster(node: &Value) -> Result<Roster, String> {
    name(node)?;
    let mut revision = None;
    let mut members = Vec::new();
    for child in children(node, "github-scope")? {
        if child.get("children").is_some() { return Err("scope children take no body".into()); }
        let child_name = child["name"].as_str().ok_or("scope child has no name")?;
        if child_name == "revision" {
            if revision.is_some() || child.get("properties").and_then(Value::as_object).is_some_and(|properties| !properties.is_empty()) {
                return Err("scope revision is repeated or has properties".into());
            }
            revision = Some(name(child)?.to_owned());
        } else if child_name == "repository" {
            let properties = child.get("properties").and_then(Value::as_object);
            if properties.is_some_and(|properties| properties.keys().any(|key| key != "from-ms" && key != "until-ms")) {
                return Err("scope repository takes only from-ms and until-ms".into());
            }
            let start = child["properties"]["from-ms"].as_u64().ok_or("scope repository needs explicit effective from-ms")?;
            let end = child["properties"].get("until-ms").map(|value| value.as_u64().ok_or("until-ms must be an integer")).transpose()?;
            let repository_id = name(child)?.to_owned();
            if end.is_some_and(|end| end <= start) || members.iter().any(|member: &RosterMember| {
                member.repository_id == repository_id && member.effective_until_ms.unwrap_or(u64::MAX) > start && end.unwrap_or(u64::MAX) > member.effective_from_ms
            }) { return Err("scope membership intervals overlap or have invalid bounds".into()); }
            members.push(RosterMember { repository_id, effective_from_ms: start, effective_until_ms: end });
        } else { return Err(format!("unknown scope child {child_name}")); }
    }
    if members.is_empty() { return Err("scope needs repository membership".into()); }
    Ok(Roster { revision: revision.ok_or("scope needs an explicit revision")?, members })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roster_requires_authored_dates_and_nonoverlapping_intervals() {
        let mut roster = json!({"name":"github-scope","arguments":["sample"],"children":[
            {"name":"revision","arguments":["one"]},
            {"name":"repository","arguments":["repo"],"properties":{"from-ms":100}}
        ]});
        assert!(parse_roster(&roster).is_ok());
        roster["children"][1]["properties"] = json!({});
        assert!(parse_roster(&roster).is_err());
        roster["children"][1]["properties"] = json!({"from-ms":100});
        let duplicate = roster["children"][1].clone();
        roster["children"].as_array_mut().unwrap().push(duplicate);
        assert!(parse_roster(&roster).is_err());
    }

    #[test]
    fn disabled_source_bootstraps_without_authority_but_enabling_requires_it() {
        let mut source = json!({"name":"github-source","arguments":["sample"],"children":[
            {"name":"host","arguments":["github.com"]},
            {"name":"generation","arguments":[1]},
            {"name":"repository","arguments":["acme/garden"]},
            {"name":"enabled","arguments":[false]},
            {"name":"checkpoint-every-ms","arguments":[60000]},
            {"name":"replicate-private","arguments":[false]}
        ]});
        let disabled = parse_source(&source).unwrap();
        assert_eq!(disabled.authority_grant, None);
        assert!(serde_json::to_value(&disabled).unwrap().get("authority_grant").is_none());
        source["children"][3]["arguments"] = json!([true]);
        assert_eq!(parse_source(&source).unwrap_err(), "enabled github-source requires an authority-grant");
        source["children"].as_array_mut().unwrap().push(json!({"name":"authority-grant","arguments":["grant-one"]}));
        assert_eq!(parse_source(&source).unwrap().authority_grant.as_deref(), Some("grant-one"));
        for invalid in [json!(""), json!(false), Value::Null] {
            source["children"][6]["arguments"] = json!([invalid]);
            assert!(parse_source(&source).is_err());
            source["children"][3]["arguments"] = json!([false]);
            assert!(parse_source(&source).is_err(), "disabled sources cannot use an invalid grant sentinel");
            source["children"][3]["arguments"] = json!([true]);
        }
    }
}
