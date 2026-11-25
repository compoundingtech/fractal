use std::{any::Any, fmt};
use fractal_wire::Error;

/// Preserve SQLite contention and already-typed storage errors without graph read budgets.
pub(crate) fn internal(error: impl fmt::Display + 'static) -> Error {
    let source = &error as &dyn Any;
    let sqlite = source.downcast_ref::<rusqlite::Error>().or_else(|| {
        source.downcast_ref::<anyhow::Error>()?.downcast_ref::<rusqlite::Error>()
    });
    if let Some(rusqlite::Error::SqliteFailure(code, _)) = sqlite {
        let kind = match code.code {
            rusqlite::ErrorCode::DatabaseBusy => Some("database-busy"),
            rusqlite::ErrorCode::DatabaseLocked => Some("database-locked"),
            _ => None,
        };
        if let Some(kind) = kind {
            return Error::new(kind, error.to_string()).with_detail("sqlite_extended_code", code.extended_code);
        }
    }
    let stored = source.downcast_ref::<Error>().or_else(|| {
        source.downcast_ref::<anyhow::Error>()?.downcast_ref::<Error>()
    });
    if let Some(stored) = stored.filter(|stored| stored.is_sqlite_contention()) {
        return Error { code: stored.code, message: error.to_string(), details: stored.details.clone() };
    }
    Error::new("internal", error.to_string())
}
