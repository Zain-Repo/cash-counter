use cash_counter::database::{Database, Direction, NewEntry};

fn setup() -> (Database, i64) {
    let mut db = Database::in_memory().unwrap();
    let register = db.create_register("Front desk", "CAD").unwrap();
    let session = db.open_session(register, "2026-10-08", 10000).unwrap();
    (db, session)
}

fn entry(direction: Direction, amount: i64, key: &str) -> NewEntry<'_> {
    NewEntry {
        direction,
        amount,
        reason: "Daily movement",
        request_key: key,
    }
}

#[test]
fn movements_update_balance_and_closing_difference() {
    let (mut db, session) = setup();
    db.record_entries(
        session,
        &[
            entry(Direction::In, 2500, "sale"),
            entry(Direction::Out, 700, "expense"),
        ],
    )
    .unwrap();
    let balance = db.balance(session).unwrap();
    assert_eq!(
        (balance.cash_in, balance.cash_out, balance.expected),
        (2500, 700, 11800)
    );
    db.close_session(session, 11750).unwrap();
    assert_eq!(db.balance(session).unwrap().difference, Some(-50));
    assert!(
        db.record_entries(session, &[entry(Direction::In, 1, "late")])
            .is_err()
    );
    assert!(db.close_session(session, 11750).is_err());
}

#[test]
fn failed_batch_rolls_back_entries_and_totals() {
    let (mut db, session) = setup();
    assert!(
        db.record_entries(
            session,
            &[
                entry(Direction::In, 500, "first"),
                entry(Direction::Out, 0, "invalid")
            ]
        )
        .is_err()
    );
    assert_eq!(db.balance(session).unwrap().expected, 10000);
    assert!(
        db.entries(session, Direction::In, 0, 100)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn retry_keys_prevent_double_counting_and_detect_conflicts() {
    let (mut db, session) = setup();
    let sale = entry(Direction::In, 500, "retry");
    let first = db.record_entries(session, &[sale]).unwrap();
    let second = db
        .record_entries(session, &[entry(Direction::In, 500, "retry")])
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(db.balance(session).unwrap().cash_in, 500);
    assert!(
        db.record_entries(session, &[entry(Direction::In, 600, "retry")])
            .is_err()
    );
}

#[test]
fn validates_dates_foreign_keys_and_open_session_uniqueness() {
    let (mut db, session) = setup();
    assert!(db.open_session(999, "2026-10-08", 0).is_err());
    let register = db.create_register("Other", "CAD").unwrap();
    for date in ["2026-02-30", "invalid", "2026-2-01"] {
        assert!(db.open_session(register, date, 0).is_err());
    }
    assert!(db.open_session(1, "2026-10-09", 0).is_err());
    db.close_session(session, 10000).unwrap();
    assert!(db.open_session(1, "2026-10-08", 0).is_err());
    assert!(db.open_session(1, "2026-10-09", 0).is_ok());
}

#[test]
fn integer_overflow_rolls_back_without_rounding() {
    let (mut db, session) = setup();
    assert!(
        db.record_entries(session, &[entry(Direction::In, i64::MAX, "overflow")])
            .is_err()
    );
    assert_eq!(db.balance(session).unwrap().expected, 10000);
}

#[test]
fn persists_across_reopening_and_separate_connections() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("register.sqlite3");
    let mut first = Database::open(&path).unwrap();
    let register = first.create_register("Desk", "CAD").unwrap();
    let session = first.open_session(register, "2026-10-08", 0).unwrap();
    let mut second = Database::open(&path).unwrap();
    first
        .record_entries(session, &[entry(Direction::In, 100, "one")])
        .unwrap();
    second
        .record_entries(session, &[entry(Direction::Out, 30, "two")])
        .unwrap();
    drop(first);
    drop(second);
    assert_eq!(
        Database::open(&path)
            .unwrap()
            .balance(session)
            .unwrap()
            .expected,
        70
    );
}

#[test]
fn large_history_has_exact_totals_and_keyset_pages() {
    let (mut db, session) = setup();
    let keys: Vec<_> = (0..50000).map(|i| format!("sale-{i}")).collect();
    let batch: Vec<_> = keys
        .iter()
        .map(|key| entry(Direction::In, 1, key))
        .collect();
    db.record_entries(session, &batch).unwrap();
    assert_eq!(db.balance(session).unwrap().expected, 60000);
    let first = db.entries(session, Direction::In, 0, 25).unwrap();
    let second = db
        .entries(session, Direction::In, first.last().unwrap().id, 25)
        .unwrap();
    assert_eq!(first.len(), 25);
    assert_eq!(second.len(), 25);
    assert!(first.last().unwrap().id < second[0].id);
}

#[test]
fn user_text_is_stored_as_data() {
    let (mut db, session) = setup();
    let reason = "'; DROP TABLE cash_in; --";
    db.record_entries(
        session,
        &[NewEntry {
            reason,
            ..entry(Direction::In, 1, "quoted")
        }],
    )
    .unwrap();
    assert_eq!(
        db.entries(session, Direction::In, 0, 10).unwrap()[0].reason,
        reason
    );
}

#[test]
fn simultaneous_writers_do_not_lose_updates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("concurrent.sqlite3");
    let mut db = Database::open(&path).unwrap();
    let register = db.create_register("Desk", "CAD").unwrap();
    let session = db.open_session(register, "2026-10-08", 0).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|worker| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = Database::open(path).unwrap();
                barrier.wait();
                for index in 0..100 {
                    db.record_entries(
                        session,
                        &[entry(Direction::In, 1, &format!("{worker}-{index}"))],
                    )
                    .unwrap();
                }
                db.record_entries(session, &[entry(Direction::In, 10, "shared")])
                    .unwrap();
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(db.balance(session).unwrap().expected, 210);
}

#[test]
fn cumulative_overflow_and_invalid_counts_are_rejected() {
    let (mut db, session) = setup();
    db.record_entries(session, &[entry(Direction::Out, i64::MAX, "maximum")])
        .unwrap();
    assert!(
        db.record_entries(session, &[entry(Direction::Out, 1, "too-much")])
            .is_err()
    );
    assert!(db.close_session(session, i64::MAX).is_err());
    assert!(db.close_session(session, -1).is_err());
    assert_eq!(db.balance(session).unwrap().counted, None);
}

#[test]
fn invalid_page_bounds_and_empty_text_are_rejected() {
    let (mut db, session) = setup();
    assert!(db.entries(session, Direction::In, -1, 10).is_err());
    assert!(db.entries(session, Direction::In, 0, 0).is_err());
    assert!(db.entries(session, Direction::In, 0, 1001).is_err());
    assert!(
        db.record_entries(
            session,
            &[NewEntry {
                reason: "   ",
                ..entry(Direction::In, 1, "blank")
            }]
        )
        .is_err()
    );
    assert!(
        db.record_entries(session, &[entry(Direction::In, 1, "")])
            .is_err()
    );
}
