use cash_counter::database::Database;
use rusqlite::Connection;

#[test]
fn version_one_upgrades_without_losing_financial_records() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("version-one.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!(
            "../src/database/migrations/001_cash_register.sql"
        ))
        .unwrap();
    connection
        .execute(
            "INSERT INTO registers(name, currency) VALUES ('Desk', 'CAD')",
            [],
        )
        .unwrap();
    connection.execute("INSERT INTO register_sessions(register_id, business_date, opening) VALUES (1, '2026-10-08', 10000)", []).unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);
    let db = Database::open(&path).unwrap();
    assert_eq!(db.balance(1).unwrap().expected, 10000);
    drop(db);
    let connection = Connection::open(path).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 3);
    let plan: String = connection.query_row(
        "EXPLAIN QUERY PLAN SELECT id FROM register_sessions WHERE register_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3", [1, 100, 25], |row| row.get(3)
    ).unwrap();
    assert!(plan.contains("register_sessions_history"), "{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
}

#[test]
fn version_two_adds_date_index_and_preserves_records_on_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("version-two.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!(
            "../src/database/migrations/001_cash_register.sql"
        ))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../src/database/migrations/002_session_history.sql"
        ))
        .unwrap();
    connection
        .execute(
            "INSERT INTO registers(name, currency) VALUES ('Desk', 'CAD')",
            [],
        )
        .unwrap();
    connection.execute("INSERT INTO register_sessions(register_id, business_date, opening) VALUES (1, '2026-10-08', 10000)", []).unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    drop(connection);
    for _ in 0..2 {
        let db = Database::open(&path).unwrap();
        assert_eq!(db.balance(1).unwrap().expected, 10000);
    }
    let connection = Connection::open(path).unwrap();
    let plan: String = connection.query_row(
        "EXPLAIN QUERY PLAN SELECT id FROM register_sessions WHERE business_date BETWEEN ?1 AND ?2",
        ["2026-10-01", "2026-10-31"], |row| row.get(3),
    ).unwrap();
    assert!(plan.contains("register_sessions_business_date"), "{plan}");
    assert!(!plan.contains("SCAN"), "{plan}");
}

#[test]
fn closing_rejects_an_outdated_reviewed_balance() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("review.sqlite3");
    let mut first = Database::open(&path).unwrap();
    let register = first.create_register("Desk", "CAD").unwrap();
    let session = first.open_session(register, "2026-10-08", 10000).unwrap();
    let mut second = Database::open(path).unwrap();
    second
        .record_entries(
            session,
            &[cash_counter::database::NewEntry {
                direction: cash_counter::database::Direction::In,
                amount: 100,
                reason: "Sale",
                request_key: "concurrent-sale",
            }],
        )
        .unwrap();
    assert!(
        first
            .close_session_if_unchanged(session, 10000, 10000)
            .is_err()
    );
    assert_eq!(first.balance(session).unwrap().counted, None);
    first
        .close_session_if_unchanged(session, 10100, 10100)
        .unwrap();
}
