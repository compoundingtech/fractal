use fractal_jobs::host_authority::{
    AuthenticatedJobBinding, HostAuthority, HostAuthorityDecision, HostAuthorityEvidence,
    JobHostBinding, decide_host_authority,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    name: String,
    known: bool,
    current: bool,
    verified: bool,
    authority_host: String,
    declaration: String,
    existing: Option<JobHostBinding>,
    signed_binding: JobHostBinding,
    origin: String,
    signer: String,
    local: String,
    expected: HostAuthorityDecision,
}

#[test]
fn host_authority_fixture_table() {
    let binding = JobHostBinding {
        job_id: "job/fixture-one".into(),
        execution_host: "fixture-alpha".into(),
        authority_declaration: "claim/membership-one".into(),
    };
    let table: serde_json::Value =
        serde_json::from_str(include_str!("host_authority.json")).unwrap();
    for case in table["cases"].as_array().unwrap() {
        let mut fields = table["base"].as_object().unwrap().clone();
        fields.extend(case.as_object().unwrap().clone());
        let fixture: Fixture = serde_json::from_value(fields.into()).unwrap();
        let evidence = HostAuthorityEvidence {
            authority: fixture.known.then_some(HostAuthority {
                host: &fixture.authority_host,
                declaration: &fixture.declaration,
                current: fixture.current,
            }),
            authenticated: fixture.verified.then_some(AuthenticatedJobBinding {
                binding: &fixture.signed_binding,
                origin: &fixture.origin,
                signer: &fixture.signer,
            }),
            existing_binding: fixture.existing.as_ref(),
        };
        assert_eq!(
            decide_host_authority(&binding, &fixture.local, &evidence),
            fixture.expected,
            "{}",
            fixture.name
        );
        let wire = serde_json::to_string(&fixture.expected).unwrap();
        assert_eq!(
            serde_json::from_str::<HostAuthorityDecision>(&wire).unwrap(),
            fixture.expected
        );
    }
    let wire = serde_json::to_string(&binding).unwrap();
    assert_eq!(
        serde_json::from_str::<JobHostBinding>(&wire).unwrap(),
        binding
    );
}
