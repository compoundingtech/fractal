use super::*;

#[test]
fn cold_version_slower_than_five_seconds_reaches_exact_cached_measurement() {
    // No contract subprocess, model fixture, credentials or network: any invocation other
    // than --version fails. The exact cached measurement can only be reached after version.
    for driver in [Driver::Omp, Driver::OpenCode] {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("producer");
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\n[ \"$1\" = --version ] || exit 99\n[ ! -e \"$HOME/ready\" ] || exit 98\nsleep 6\ntouch \"$HOME/ready\"\nprintf '{}/99.42.7\\n'\n",
                driver.as_str()
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let adapter = directory.path().join("adapter");
        fs::write(&adapter, "fixture adapter").unwrap();
        let adapter = (driver == Driver::Omp).then_some(adapter.as_path());
        let installed = installed_identity(binary.to_str().unwrap(), driver, adapter).unwrap();
        let identity = Identity {
            driver,
            version: "99.42.7".into(),
            executable: installed.executable,
            executable_digest: installed.executable_digest,
            adapter_digest: installed.adapter_digest,
            probe_digest: probe_digest(),
        };
        let cache = directory.path().join("cache");
        fs::create_dir(&cache).unwrap();
        let path = cache.join(format!(
            "{}-{}.json",
            driver.as_str(),
            digest(&serde_json::to_vec(&identity).unwrap())
        ));
        let record = serde_json::to_vec(&Record {
            schema: SCHEMA.into(),
            identity,
            observed_at: 1,
            measurements: checks(driver)
                .iter()
                .map(|&check| measurement(check, true))
                .collect(),
            probe_failure: None,
            diagnostic: None,
        })
        .unwrap();
        fs::write(&path, &record).unwrap();
        let started = Instant::now();
        let admission = admit_at(binary.to_str().unwrap(), driver, adapter, &cache).unwrap();
        assert!(started.elapsed() >= Duration::from_secs(6));
        assert!(admission.passed(), "{driver:?}: {admission:?}");
        assert_eq!(admission.version.as_deref(), Some("99.42.7"));
        assert_eq!(admission.cache_path.as_ref(), Some(&path));
        assert_eq!(fs::read(&path).unwrap(), record);
    }
}
