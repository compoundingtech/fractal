//! What the fleet's `notice-owner` declaration says: the one member whose daemon writes notice
//! snapshots, bound to one admitted incarnation of that member by its member key, with the
//! author's qualification that the member is always on. The declaration is a singleton: its
//! subject is fixed, so a handover edits it and two members cannot both hold it.
//!
//! ```kdl
//! notice-owner {
//!   member "alder"
//!   member-key "BASE64URL"
//!   always-on
//! }
//! ```

use serde_json::Value;

use crate::model::FractalError;

/// The declaration kind and subject family.
pub const KIND: &str = "notice-owner";
/// The singleton subject every `notice-owner` declaration writes.
pub const SUBJECT: &str = "notice-owner/fleet";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeOwner {
    /// The fleet member name, which is also the `origin` of every claim its daemon writes.
    pub member: String,
    /// The public key of the bound incarnation of that member.
    pub member_key: String,
}

/// Read a canonical declaration body strictly: a missing, repeated or unknown child, or a
/// missing `always-on`, is an error, never a default.
pub fn parse(desired: &Value) -> Result<NoticeOwner, FractalError> {
    let fail = |message: &str| FractalError::new("invalid-notice-owner", format!("notice-owner: {message}"));
    if desired["name"] != KIND || desired.get("arguments").is_some() || desired.get("properties").is_some() {
        return Err(fail("takes no name or properties; its subject is notice-owner/fleet"));
    }
    let children = desired.get("children").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let mut member = None;
    let mut member_key = None;
    let mut always_on = false;
    for child in children {
        let name = child["name"].as_str().unwrap_or_default();
        let repeated = match name {
            "member" => member.replace(one_string(child, name, &fail)?).is_some(),
            "member-key" => member_key.replace(one_string(child, name, &fail)?).is_some(),
            "always-on" => {
                if child.as_object().is_some_and(|node| node.len() != 1) {
                    return Err(fail("`always-on` is a bare flag"));
                }
                std::mem::replace(&mut always_on, true)
            }
            _ => return Err(fail(&format!("unknown child `{name}`"))),
        };
        if repeated {
            return Err(fail(&format!("`{name}` repeats")));
        }
    }
    let member = member.ok_or_else(|| fail("needs `member`"))?;
    if member.is_empty() || !member.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) {
        return Err(fail("`member` must name one fleet member"));
    }
    let member_key = member_key.ok_or_else(|| fail("needs `member-key`, the bound incarnation's public key"))?;
    if member_key.is_empty() || member_key.len() > 128
        || !member_key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(fail("`member-key` must be one member public key"));
    }
    if !always_on {
        return Err(fail("needs `always-on`: only an always-on member may own notices"));
    }
    Ok(NoticeOwner { member, member_key })
}

fn one_string(node: &Value, name: &str, fail: &dyn Fn(&str) -> FractalError) -> Result<String, FractalError> {
    match (node.get("arguments").and_then(Value::as_array).map(Vec::as_slice), node.as_object().map(|n| n.len())) {
        (Some([Value::String(value)]), Some(2)) => Ok(value.clone()),
        _ => Err(fail(&format!("`{name}` needs one string"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration_kdl::canonical_node;

    fn read(source: &str) -> Result<NoticeOwner, FractalError> {
        let document: kdl::KdlDocument = source.parse().unwrap();
        parse(&canonical_node(&document.nodes()[0]).unwrap())
    }

    #[test]
    fn a_notice_owner_binds_one_member_incarnation_qualified_always_on() {
        assert_eq!(
            read("notice-owner { member \"alder\"; member-key \"a-b_C1\"; always-on }").unwrap(),
            NoticeOwner { member: "alder".into(), member_key: "a-b_C1".into() }
        );
        for broken in [
            "notice-owner \"alder\" { member \"alder\"; member-key \"k\"; always-on }",
            "notice-owner { member \"alder\"; member-key \"k\" }",
            "notice-owner { member-key \"k\"; always-on }",
            "notice-owner { member \"alder\"; always-on }",
            "notice-owner { member \"alder\"; member \"birch\"; member-key \"k\"; always-on }",
            "notice-owner { member \"host/alder\"; member-key \"k\"; always-on }",
            "notice-owner { member \"alder\"; member-key \"k\"; always-on #true }",
            "notice-owner { member \"alder\"; member-key \"k\"; always-on; standby \"birch\" }",
        ] {
            assert_eq!(read(broken).unwrap_err().code, "invalid-notice-owner", "{broken}");
        }
    }
}
