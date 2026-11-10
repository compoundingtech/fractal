//! A publication carries the bytes of each program bundle its missions declare
//! (docs/st3/program-engine.md §4): the CLI reads every declared path, checks it against the
//! declared sha256 and sends the bytes, so the daemon never reads the author's path.
use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use base64::Engine as _;
use fractal_daemon::model::{NormalizedIntent, ProgramSpec};
use sha2::{Digest as _, Sha256};

/// Every program bundle the intent text declares, base64 by sha256.
pub(crate) fn program_bundles(kdl: &str) -> Result<BTreeMap<String, String>> {
    // The most permissive parse: it only finds the programs; the daemon validates the intent.
    let intent = fractal_store::graph::parse_intent_with_owner(kdl, "local", None, true, true)
        .context("find the intent's program bundles")?;
    let mut bundles = BTreeMap::new();
    for program in programs(&intent) {
        let bytes = read_program_bundle(program)?;
        bundles.insert(program.bundle_hash.clone(), base64::engine::general_purpose::STANDARD.encode(bytes));
    }
    Ok(bundles)
}

pub(crate) fn programs(intent: &NormalizedIntent) -> impl Iterator<Item = &ProgramSpec> {
    intent.missions.values().flat_map(|mission| mission.programs()).map(|(_, program)| program)
}

/// The bundle's bytes, refused unless they are exactly the ones the mission declares.
pub(crate) fn read_program_bundle(program: &ProgramSpec) -> Result<Vec<u8>> {
    let bytes = std::fs::read(&program.bundle).with_context(|| format!("read program bundle {}", program.bundle))?;
    let actual = hex::encode(Sha256::digest(&bytes));
    anyhow::ensure!(
        actual == program.bundle_hash,
        "program bundle {} has sha256 {actual}, but the mission declares {}",
        program.bundle,
        program.bundle_hash
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_publication_carries_exactly_the_declared_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("land.mjs");
        std::fs::write(&bundle, b"export default {}").unwrap();
        let hash = hex::encode(Sha256::digest(b"export default {}"));
        let kdl = |hash: &str| {
            format!(
                r#"version 2
                mission "land" state="ready" {{ goal "Land."; step "program" {{ program "{}" sha256="{hash}"; }} }}
                mission "outer" state="ready" {{ goal "Nest."; step "inner" {{ mission "inner" state="ready" {{ goal "In."; step "program" {{ program "{}" sha256="{hash}"; }}; }}; }} }}"#,
                bundle.display(),
                bundle.display()
            )
        };
        let bundles = program_bundles(&kdl(&hash)).unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&bundles[&hash]).unwrap(), b"export default {}");
        let other = "0".repeat(64);
        let error = program_bundles(&kdl(&other)).unwrap_err().to_string();
        assert!(error.contains(&hash) && error.contains(&other) && error.contains("land.mjs"), "{error}");
        assert!(program_bundles("version 2\nmission \"plain\" state=\"ready\" { goal \"None.\"; step \"work\" { agentless; } }").unwrap().is_empty());
    }
}
