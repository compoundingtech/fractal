//! The server's removed legacy handlers stay out of api.rs and out of every route-family file.

use std::collections::BTreeSet;

#[test]
fn removed_legacy_handlers_are_absent_from_api_and_its_route_families() {
    // api.rs declares each route family right after its "Route families" comment; the list must
    // name every one, so a removed handler cannot come back in a family file unseen.
    let families = [
        ("agents", /* sqlite-large-value: allow-for-now lv-hf343ed6a9b5d owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("agents.rs")),
        ("client_reads", /* sqlite-large-value: allow-for-now lv-h7507c9c06532 owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("client_reads.rs")),
        ("fleet", /* sqlite-large-value: allow-for-now lv-hd82cb2e4adf3 owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("fleet.rs")),
        ("launches", /* sqlite-large-value: allow-for-now lv-hb3cca75d21d1 owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("launches.rs")),
        ("messages", /* sqlite-large-value: allow-for-now lv-h43c19aca2faa owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("messages.rs")),
        ("operations", /* sqlite-large-value: allow-for-now lv-h5489ceb8d6c0 owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("operations.rs")),
        ("records", /* sqlite-large-value: allow-for-now lv-h2416cabf721b owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("records.rs")),
        ("replication", /* sqlite-large-value: allow-for-now lv-hfeca42bbeb2a owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("replication.rs")),
        ("runs", /* sqlite-large-value: allow-for-now lv-h610d826d833a owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("runs.rs")),
        ("sessions", /* sqlite-large-value: allow-for-now lv-hfdaa0b44a23e owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("sessions.rs")),
        ("work", /* sqlite-large-value: allow-for-now lv-h17a0db59ee8b owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("work.rs")),
    ];
    let api = /* sqlite-large-value: allow-for-now lv-h2f6ac5ab5c4f owner=fractal_api_core::api::legacy_handler_guard_tests TODO=trace-statement-and-value-representation */ include_str!("../api.rs");
    let declared: BTreeSet<&str> = api
        .split("// Route families:")
        .nth(1)
        .unwrap()
        .lines()
        .skip(2)
        .map_while(|line| {
            line.strip_prefix("pub ")
                .unwrap_or(line)
                .strip_prefix("mod ")?
                .strip_suffix(';')
        })
        .collect();
    assert_eq!(
        declared,
        families.iter().map(|(name, _)| *name).collect::<BTreeSet<_>>(),
        "every route family api.rs declares is scanned"
    );
    let server_source = std::iter::once(api)
        .chain(families.iter().map(|(_, source)| *source))
        .collect::<String>();
    for handler in [
        "watch_resource",
        "unwatch_resource",
        "refresh_resource",
        "reset_runtime",
        "quick_claude",
        "quick_codex",
        "start_mission_run",
    ] {
        assert!(
            !server_source.contains(&format!("fn {handler}(")),
            "legacy server handler {handler} remains compiled"
        );
    }
}
