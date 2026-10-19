//! The tables of the node-local git change index. The index methods live in st3-github; the
//! schema installs with every store, so it stays with the store.

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h8bb33b1f6576 owner=fractal_store::store::git_change_tables TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS local_git_change_records (
    repository TEXT NOT NULL,
    object_id TEXT NOT NULL,
    change_id TEXT NOT NULL,
    record_json TEXT NOT NULL,
    indexed_at TEXT NOT NULL,
    PRIMARY KEY(repository, object_id)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_git_change_latest (
    repository TEXT NOT NULL,
    change_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    PRIMARY KEY(repository, change_id)
) WITHOUT ROWID;
"#;
