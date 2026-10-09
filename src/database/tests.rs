use super::*;

fn setup() -> (Database, i64) {
    let mut db = Database::in_memory().unwrap();
    let register = db.create_register("Desk", "CAD").unwrap();
    let session = db.open_session(register, "2026-10-08", 0).unwrap();
    (db, session)
}

#[test]
fn schema_guards_history_even_for_direct_sql() {
    let (mut db, session) = setup();
    db.record_entries(
        session,
        &[NewEntry {
            direction: Direction::In,
            amount: 100,
            reason: "Sale",
            request_key: "one",
        }],
    )
    .unwrap();
    for sql in [
        "UPDATE cash_in SET amount = 200",
        "DELETE FROM cash_in",
        "INSERT OR REPLACE INTO cash_in(id, session_id, amount, reason, request_key) VALUES (1, 1, 200, 'Replacement', 'one')",
        "DELETE FROM register_sessions",
        "UPDATE register_sessions SET opening = 200",
    ] {
        assert!(
            db.connection.execute(sql, []).is_err(),
            "Unexpected success: {sql}"
        );
    }
    assert_eq!(db.balance(session).unwrap().expected, 100);
}

#[test]
fn balance_and_history_queries_use_indexes() {
    let (db, _) = setup();
    let plan: String = db
        .connection
        .query_row(
            "EXPLAIN QUERY PLAN SELECT expected FROM register_sessions WHERE id = ?1",
            [1],
            |row| row.get(3),
        )
        .unwrap();
    assert!(plan.contains("INTEGER PRIMARY KEY"), "{plan}");
    for table in ["cash_in", "cash_out"] {
        // Table names here are fixed test constants, never user input.
        let sql = format!(
            "EXPLAIN QUERY PLAN SELECT id, amount FROM {table} WHERE session_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3"
        );
        let plan: String = db
            .connection
            .query_row(&sql, [1, 0, 25], |row| row.get(3))
            .unwrap();
        assert!(plan.contains(&format!("{table}_session_id")), "{plan}");
        assert!(!plan.contains("SCAN"), "{plan}");
    }
}

#[test]
fn migration_is_atomic_on_failure() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE cash_out(id INTEGER PRIMARY KEY)")
        .unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(
        transaction
            .execute_batch(include_str!("migrations/001_cash_register.sql"))
            .is_err()
    );
    drop(transaction);
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'registers'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn rejects_future_schema() {
    let connection = Connection::open_in_memory().unwrap();
    connection.pragma_update(None, "user_version", 3).unwrap();
    assert!(Database::initialize(connection).is_err());
}

#[test]
fn database_passes_integrity_and_foreign_key_checks() {
    let (db, _) = setup();
    let integrity: String = db
        .connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    assert!(
        !db.connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
}
