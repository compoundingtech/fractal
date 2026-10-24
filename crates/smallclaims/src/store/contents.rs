//! What a store holds, compared across stores in initialization tests.

/// What a store holds: its catalog, a few header settings, and every row of every table. An
/// integer within a day of now in Unix ms, stored as an integer or as text, is when its row was
/// written, so it reads as `now`.
pub fn stored(path: &std::path::Path) -> (Vec<String>, Vec<String>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let shown = |value: rusqlite::types::Value| match value {
        rusqlite::types::Value::Integer(at) if (at - now).abs() < 86_400_000 => "now".to_owned(),
        rusqlite::types::Value::Text(at)
            if at.parse::<i64>().is_ok_and(|at| (at - now).abs() < 86_400_000) =>
        {
            "now".to_owned()
        }
        value => format!("{value:?}"),
    };
    let connection = rusqlite::Connection::open(path).unwrap();
    let catalog = connection
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY type, name")
        .unwrap()
        .query_map([], |row| {
            Ok(format!(
                "{}|{}|{}|{}",
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?.unwrap_or_default()
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut rows = Vec::new();
    for pragma in ["auto_vacuum", "user_version", "journal_mode"] {
        let value: String = connection
            .query_row(/* sqlite-large-value: allow-for-now lv-hf828e05fb675 owner=smallclaims::store::contents TODO=trace-statement-and-value-representation */ &format!("SELECT CAST({pragma} AS TEXT) FROM pragma_{pragma}"), [], |row| row.get(0))
            .unwrap();
        rows.push(format!("pragma {pragma}={value}"));
    }
    let tables = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT GLOB 'sqlite_*' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for table in tables {
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h81b9c8a8b7b9 owner=smallclaims::store::contents TODO=trace-statement-and-value-representation */ &format!("SELECT * FROM \"{table}\"")).unwrap();
        let columns = statement.column_count();
        let mut table_rows = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|index| row.get::<_, rusqlite::types::Value>(index).map(shown))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap()
            .map(|row| format!("{table}: {}", row.unwrap().join(", ")))
            .collect::<Vec<_>>();
        table_rows.sort();
        rows.extend(table_rows);
    }
    (catalog, rows)
}

/// The entries only one of two stores holds; empty when they hold the same.
pub fn differences(left: &(Vec<String>, Vec<String>), right: &(Vec<String>, Vec<String>)) -> Vec<String> {
    let mut only = Vec::new();
    for (side, mine, theirs) in [("left", left, right), ("right", right, left)] {
        for entry in mine.0.iter().chain(mine.1.iter()) {
            if !theirs.0.contains(entry) && !theirs.1.contains(entry) {
                only.push(format!("only {side}: {entry}"));
            }
        }
    }
    only
}

/// Run `check` for every index below `count`, spread over a few threads. Each index works on its
/// own store.
pub fn each_in_parallel(count: usize, check: impl Fn(usize) + Sync) {
    let workers = std::thread::available_parallelism().map_or(4, |cores| cores.get().min(8));
    std::thread::scope(|scope| {
        for worker in 0..workers {
            let check = &check;
            scope.spawn(move || (worker..count).step_by(workers).for_each(check));
        }
    });
}

/// Where an initialization test puts the stores it opens and throws away: a memory filesystem
/// where there is one. These tests check what a store holds, not what reaches a disk, and on a
/// disk nearly all of a store's open is spent waiting on fsync.
pub fn scratch_root() -> std::path::PathBuf {
    match std::path::Path::new("/dev/shm") {
        shm if shm.is_dir() => shm.to_owned(),
        _ => std::env::temp_dir(),
    }
}
