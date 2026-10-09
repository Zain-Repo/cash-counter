use cash_counter::{
    database::Direction,
    register::{Operation, RegisterService, Selection},
};

#[test]
fn workspace_persists_a_complete_daily_workflow() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace.sqlite3");
    let mut service = RegisterService::open(&path).unwrap();
    let selection = Selection::default();
    let initial = service
        .execute(&Operation::Refresh, &selection)
        .snapshot
        .unwrap();
    assert!(initial.registers.is_empty());
    let setup = service
        .execute(
            &Operation::CreateRegister {
                name: "Main register".into(),
            },
            &selection,
        )
        .snapshot
        .unwrap();
    let register = setup.register.unwrap();
    assert_eq!(register.currency, "CAD");
    let opened = service
        .execute(
            &Operation::Open {
                register: register.id,
                date: "2026-10-08".into(),
                opening: 10000,
            },
            &selection,
        )
        .snapshot
        .unwrap();
    let session = opened.session.unwrap().id;
    let selection = Selection {
        register: Some(register.id),
        session: Some(session),
        ..Selection::default()
    };
    let operation = Operation::Record {
        session,
        direction: Direction::In,
        amount: 2000,
        reason: "Cash sale".into(),
        request_key: "sale-001".into(),
    };
    assert!(service.execute(&operation, &selection).applied);
    assert!(service.execute(&operation, &selection).applied);
    let outgoing = service
        .execute(
            &Operation::Record {
                session,
                direction: Direction::Out,
                amount: 500,
                reason: "Petty cash".into(),
                request_key: "expense-001".into(),
            },
            &selection,
        )
        .snapshot
        .unwrap();
    assert_eq!(outgoing.balance.unwrap().expected, 11500);
    assert_eq!(outgoing.entries[0].reason, "Petty cash");
    let closed = service
        .execute(
            &Operation::Close {
                session,
                counted: 11450,
                expected: 11500,
            },
            &selection,
        )
        .snapshot
        .unwrap();
    assert_eq!(closed.balance.unwrap().difference, Some(-50));
    assert!(closed.session.unwrap().closed);
    drop(service);
    let reopened = RegisterService::open(path)
        .unwrap()
        .execute(&Operation::Refresh, &selection)
        .snapshot
        .unwrap();
    assert_eq!(reopened.balance.unwrap().counted, Some(11450));
}

#[test]
fn workspace_history_is_bounded_and_paged_without_overlap() {
    let directory = tempfile::tempdir().unwrap();
    let mut service = RegisterService::open(directory.path().join("pages.sqlite3")).unwrap();
    let mut selection = Selection::default();
    let register = service
        .execute(
            &Operation::CreateRegister {
                name: "Desk".into(),
            },
            &selection,
        )
        .snapshot
        .unwrap()
        .register
        .unwrap()
        .id;
    selection.register = Some(register);
    let session = service
        .execute(
            &Operation::Open {
                register,
                date: "2026-10-08".into(),
                opening: 0,
            },
            &selection,
        )
        .snapshot
        .unwrap()
        .session
        .unwrap()
        .id;
    selection.session = Some(session);
    for index in 0..30 {
        assert!(
            service
                .execute(
                    &Operation::Record {
                        session,
                        direction: Direction::In,
                        amount: 1,
                        reason: "Sale".into(),
                        request_key: format!("sale-{index}")
                    },
                    &selection
                )
                .applied
        );
    }
    let first = service
        .execute(&Operation::Refresh, &selection)
        .snapshot
        .unwrap();
    assert_eq!(first.entries.len(), 25);
    assert!(first.more_entries);
    selection.before_entry = Some(first.entries.last().unwrap().id);
    let second = service
        .execute(&Operation::Refresh, &selection)
        .snapshot
        .unwrap();
    assert_eq!(second.entries.len(), 5);
    assert!(!second.more_entries);
    assert!(first.entries.last().unwrap().id > second.entries[0].id);
}

#[test]
fn constraint_failures_allow_correction_and_do_not_change_totals() {
    let directory = tempfile::tempdir().unwrap();
    let mut service = RegisterService::open(directory.path().join("invalid.sqlite3")).unwrap();
    let mut selection = Selection::default();
    selection.register = Some(
        service
            .execute(
                &Operation::CreateRegister {
                    name: "Desk".into(),
                },
                &selection,
            )
            .snapshot
            .unwrap()
            .register
            .unwrap()
            .id,
    );
    let session = service
        .execute(
            &Operation::Open {
                register: selection.register.unwrap(),
                date: "2026-10-08".into(),
                opening: 0,
            },
            &selection,
        )
        .snapshot
        .unwrap()
        .session
        .unwrap()
        .id;
    selection.session = Some(session);
    let reply = service.execute(
        &Operation::Record {
            session,
            direction: Direction::In,
            amount: 0,
            reason: "Sale".into(),
            request_key: "zero".into(),
        },
        &selection,
    );
    assert!(!reply.applied);
    assert!(!reply.retryable);
    assert!(reply.snapshot.is_err());
    assert_eq!(
        service
            .execute(&Operation::Refresh, &selection)
            .snapshot
            .unwrap()
            .balance
            .unwrap()
            .expected,
        0
    );
}
