//! Claim pages by subject, subject prefix and kind, each read through a claims index.
use super::*;

/// Which claims a page holds. `subject` and `subject_prefix` exclude each other; `kind` combines
/// with either.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClaimsFilter<'a> {
    pub subject: Option<&'a str>,
    pub subject_prefix: Option<&'a str>,
    pub kind: Option<&'a str>,
}

fn invalid(message: &str) -> anyhow::Error {
    FractalError::new("invalid-claims-query", message).into()
}

/// The `[lower, upper)` subject range a prefix spans; `claims_subject_index` seeks it, where a
/// `LIKE` pattern would scan.
fn prefix_range(prefix: &str) -> Result<(&str, String)> {
    let upper = prefix_upper_bound(prefix).ok_or_else(|| {
        invalid("a subject prefix must be non-empty ASCII without `*`, `?` or `[`")
    })?;
    Ok((prefix, upper))
}

/// Claims of `kinds` kinds (`?5`…) after store index `?1`, at most `?2` rows, under the subject
/// range `?3 <= subject < ?4` when `prefix`.
fn kinds_after_query(kinds: usize, prefix: bool) -> String {
    let placeholders = (0..kinds).map(|n| format!("?{}", n + 5)).collect::<Vec<_>>().join(",");
    let subject_filter = if prefix { " AND subject>=?3 AND subject<?4" } else { "" };
    format!(
        "SELECT id, store_index, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms
         FROM claims INDEXED BY claims_kind_index
         WHERE kind IN ({placeholders}) AND store_index>?1{subject_filter}
         ORDER BY store_index LIMIT ?2"
    )
}

/// How `filter` selects subjects for [`claims_page_query`], with the exact subject or the lower
/// bound (`?3`) and the prefix's upper bound (`?6`).
pub(super) fn subject_match(
    filter: ClaimsFilter<'_>,
) -> Result<(ClaimsSubject, Option<&str>, Option<String>)> {
    Ok(match (filter.subject, filter.subject_prefix) {
        (Some(_), Some(_)) => {
            return Err(invalid("`subject` and `subject_prefix` exclude each other"));
        }
        (Some(subject), None) => (ClaimsSubject::Exact, Some(subject), None),
        (None, Some(prefix)) => {
            let (lower, upper) = prefix_range(prefix)?;
            (ClaimsSubject::Range, Some(lower), Some(upper))
        }
        (None, None) => (ClaimsSubject::Any, None, None),
    })
}

impl Store {
    /// The next `limit` claims of any of `kinds` after `after_index`, ascending, optionally under
    /// `subject_prefix`. Each kind seeks `claims_kind_index` past the cursor, so a caller that keeps
    /// the cursor reads only claims newer than the last it saw.
    pub fn claims_by_kinds_after(
        &self,
        kinds: &[&str],
        subject_prefix: Option<&str>,
        after_index: u64,
        limit: usize,
    ) -> Result<ClaimsPage> {
        if kinds.is_empty() {
            return Ok(ClaimsPage { claims: Vec::new(), next_cursor: None });
        }
        let range = subject_prefix.map(prefix_range).transpose()?;
        let query = kinds_after_query(kinds.len(), range.is_some());
        let scan_limit = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let (lower, upper) = range.map_or((None, None), |(lower, upper)| (Some(lower), Some(upper)));
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&after_index, &scan_limit, &lower, &upper];
        values.extend(kinds.iter().map(|kind| kind as &dyn rusqlite::ToSql));
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h5b4b8a42e7c8 owner=fractal_store::store::claim_pages TODO=trace-statement-and-value-representation */ &query)?;
        let mut claims = statement
            .query_map(rusqlite::params_from_iter(values), claim_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let next_cursor = if claims.len() > limit {
            claims.pop();
            claims.last().map(|claim| claim.store_index)
        } else {
            None
        };
        Ok(ClaimsPage { claims, next_cursor })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANGE: &str = "custom.acme.change";
    const NOTE: &str = "custom.acme.note";

    fn append(store: &Store, subject: &str, kind: &str) -> u64 {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: None,
                fields: BTreeMap::new(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
            .store_index
    }

    fn indexes(page: &ClaimsPage) -> Vec<u64> {
        page.claims.iter().map(|claim| claim.store_index).collect()
    }

    #[test]
    fn claim_pages_filter_by_kind_subject_and_subject_prefix() {
        let store = Store::open_memory("node").unwrap();
        let a = append(&store, "custom/acme/change/1", CHANGE);
        let b = append(&store, "custom/acme/changes/x", CHANGE);
        let c = append(&store, "custom/acme/change/2", NOTE);
        let d = append(&store, "custom/other/change/1", CHANGE);
        let e = append(&store, "custom/acme/change/1", NOTE);
        let page = |filter: ClaimsFilter<'_>, after, before, descending, limit| {
            store.claims_page_matching(filter, None, after, before, descending, limit).unwrap()
        };
        let kind = ClaimsFilter { kind: Some(CHANGE), ..ClaimsFilter::default() };
        let prefix = ClaimsFilter { subject_prefix: Some("custom/acme/change/"), ..ClaimsFilter::default() };
        let subject = ClaimsFilter { subject: Some("custom/acme/change/1"), ..ClaimsFilter::default() };
        for (filter, expected) in [
            (kind, vec![a, b, d]),
            // `custom/acme/changes/x` shares the characters but not the prefix.
            (prefix, vec![a, c, e]),
            (ClaimsFilter { kind: Some(CHANGE), ..prefix }, vec![a]),
            (subject, vec![a, e]),
            (ClaimsFilter { kind: Some(NOTE), ..subject }, vec![e]),
        ] {
            assert_eq!(indexes(&page(filter, 0, None, false, 10)), expected, "{filter:?}");
            let mut descending = expected.clone();
            descending.reverse();
            assert_eq!(indexes(&page(filter, 0, None, true, 10)), descending, "{filter:?}");
        }
        let first = page(prefix, 0, None, false, 2);
        assert_eq!((indexes(&first), first.next_cursor), (vec![a, c], Some(c)));
        let rest = page(prefix, c, None, false, 2);
        assert_eq!((indexes(&rest), rest.next_cursor), (vec![e], None));
        let older = page(prefix, 0, Some(e), true, 1);
        assert_eq!((indexes(&older), older.next_cursor), (vec![c], Some(c)));

        for invalid in [
            ClaimsFilter { subject_prefix: Some("custom/acme/"), ..subject },
            ClaimsFilter { subject_prefix: Some(""), ..ClaimsFilter::default() },
            ClaimsFilter { subject_prefix: Some("custom/*"), ..ClaimsFilter::default() },
        ] {
            let error = store.claims_page_matching(invalid, None, 0, None, false, 10).unwrap_err();
            assert_eq!(error.downcast_ref::<FractalError>().unwrap().code, "invalid-claims-query");
        }
    }

    #[test]
    fn claim_page_allowlists_intersect_filters_before_pagination() {
        let store = Store::open_memory("node").unwrap();
        let denied = "custom.acme.private";
        append(&store, "custom/acme/change/1", denied);
        let a = append(&store, "custom/acme/change/1", CHANGE);
        append(&store, "custom/acme/change/1", denied);
        let b = append(&store, "custom/acme/change/2", NOTE);
        append(&store, "custom/acme/changes/outside", CHANGE);
        let c = append(&store, "custom/acme/change/1", CHANGE);
        append(&store, "custom/acme/change/1", denied);
        let prefix = ClaimsFilter {
            subject_prefix: Some("custom/acme/change/"),
            ..ClaimsFilter::default()
        };
        let allowed = [CHANGE, NOTE];
        let page = |filter, after, before, descending, limit, kinds| {
            store.claims_page_matching_with_kinds(
                filter, None, after, before, descending, limit, kinds,
            ).unwrap()
        };
        let first = page(prefix, 0, None, false, 1, Some(allowed.as_slice()));
        assert_eq!((indexes(&first), first.next_cursor), (vec![a], Some(a)));
        let rest = page(prefix, a, None, false, 10, Some(allowed.as_slice()));
        assert_eq!((indexes(&rest), rest.next_cursor), (vec![b, c], None));
        let newest = page(prefix, 0, None, true, 1, Some(allowed.as_slice()));
        assert_eq!((indexes(&newest), newest.next_cursor), (vec![c], Some(c)));
        let older = page(prefix, 0, Some(c), true, 10, Some(allowed.as_slice()));
        assert_eq!((indexes(&older), older.next_cursor), (vec![b, a], None));
        let exact = ClaimsFilter { subject: Some("custom/acme/change/1"), ..ClaimsFilter::default() };
        assert_eq!(indexes(&page(exact, 0, None, false, 10, Some(allowed.as_slice()))), vec![a, c]);
        let kind = ClaimsFilter { kind: Some(CHANGE), ..prefix };
        assert_eq!(indexes(&page(kind, 0, None, false, 10, Some(allowed.as_slice()))), vec![a, c]);
        let excluded = ClaimsFilter { kind: Some(denied), ..prefix };
        let empty = page(excluded, 0, None, false, 10, Some(allowed.as_slice()));
        assert!(empty.claims.is_empty());
        assert_eq!(empty.next_cursor, None);
        let empty = page(prefix, 0, None, false, 10, Some([].as_slice()));
        assert!(empty.claims.is_empty());
        assert_eq!(empty.next_cursor, None);
        assert_eq!(
            indexes(&page(prefix, 0, None, false, 10, None)),
            indexes(&store.claims_page_matching(prefix, None, 0, None, false, 10).unwrap()),
        );
    }

    #[test]
    fn claims_by_kinds_after_reads_newer_claims_under_a_prefix() {
        let store = Store::open_memory("node").unwrap();
        let a = append(&store, "custom/acme/change/1", CHANGE);
        append(&store, "custom/acme/changes/x", CHANGE);
        let c = append(&store, "custom/acme/change/2", NOTE);
        let d = append(&store, "custom/other/change/1", CHANGE);
        append(&store, "custom/acme/change/3", "custom.acme.other");
        let both = [CHANGE, NOTE];
        let first = store.claims_by_kinds_after(&both, Some("custom/acme/change/"), 0, 1).unwrap();
        assert_eq!((indexes(&first), first.next_cursor), (vec![a], Some(a)));
        let rest = store.claims_by_kinds_after(&both, Some("custom/acme/change/"), a, 10).unwrap();
        assert_eq!((indexes(&rest), rest.next_cursor), (vec![c], None));
        let every = store.claims_by_kinds_after(&[CHANGE], None, a, 10).unwrap();
        assert_eq!(indexes(&every).last(), Some(&d));
        assert_eq!(every.claims.len(), 2);
        assert!(store.claims_by_kinds_after(&[], None, 0, 10).unwrap().claims.is_empty());
    }

    fn plan(store: &Store, query: &str) -> String {
        let connection = store.readers.get();
        let mut statement = connection.prepare(&format!("EXPLAIN QUERY PLAN {query}")).unwrap();
        let nulls = vec![None::<String>; statement.parameter_count()];
        statement
            .query_map(rusqlite::params_from_iter(nulls), |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("; ")
    }

    /// A kind with a prefix walks `claims_kind_index` past the cursor rather than the subject
    /// range: the page comes out in store-index order and stops at its limit, and a reader that
    /// keeps its cursor touches only newer claims of that kind. A prefix alone seeks its range.
    #[test]
    fn each_claim_page_filter_seeks_its_index() {
        let store = Store::open_memory("node").unwrap();
        for descending in [false, true] {
            for (subject, kind, expected) in [
                (ClaimsSubject::Any, true, "USING INDEX claims_kind_index (kind=? AND store_index>?)"),
                (ClaimsSubject::Range, true, "USING INDEX claims_kind_index (kind=? AND store_index>?)"),
                (ClaimsSubject::Range, false, "USING INDEX claims_subject_index (subject>? AND subject<?)"),
                (ClaimsSubject::Exact, true, "USING INDEX claims_subject_kind_index (subject=? AND kind=? AND store_index>?)"),
            ] {
                let plan = plan(&store, &claims_page_query(subject, kind, descending, false));
                assert!(plan.contains(expected), "{subject:?} kind={kind}: {plan}");
                assert!(!plan.contains("SCAN claims"), "{subject:?} kind={kind}: {plan}");
            }
        }
        for prefix in [false, true] {
            let plan = plan(&store, &kinds_after_query(2, prefix));
            assert!(plan.contains("USING INDEX claims_kind_index (kind=? AND store_index>?)"), "{plan}");
            assert!(!plan.contains("SCAN claims"), "{plan}");
        }
    }
}
