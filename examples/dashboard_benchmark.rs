//! Reproducible dashboard workload against an isolated, automatically removed ledger.
use cash_counter::{
    analytics::{DashboardFilter, load_dashboard},
    database::{Database, Direction, NewEntry},
};
use std::{error::Error, io, time::Instant};

const DEFAULT_ENTRIES: usize = 10_000;
const MAX_ENTRIES: usize = 1_000_000;
const BATCH_SIZE: usize = 1_000;
const REGISTER_COUNT: usize = 8;
const DAY_COUNT: usize = 30;

fn entry_count() -> Result<usize, io::Error> {
    let mut arguments = std::env::args().skip(1);
    let count = match arguments.next() {
        Some(value) => value.parse::<usize>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Entry count must be an integer",
            )
        })?,
        None => DEFAULT_ENTRIES,
    };
    if arguments.next().is_some() || !(1..=MAX_ENTRIES).contains(&count) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Usage: cargo run --release --example dashboard_benchmark -- [1..1000000]",
        ));
    }
    Ok(count)
}

fn main() -> Result<(), Box<dyn Error>> {
    let count = entry_count()?;
    if cfg!(debug_assertions) {
        eprintln!(
            "Debug build: timings are diagnostic. Use --release for performance comparisons."
        );
    }
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("synthetic-ledger.sqlite3");
    let generation_started = Instant::now();
    let mut database = Database::open(&path)?;
    let mut generated = 0usize;
    let mut expected_in = 0i64;
    let mut expected_out = 0i64;
    let session_count = REGISTER_COUNT * DAY_COUNT;

    for register_index in 0..REGISTER_COUNT {
        let register =
            database.create_register(&format!("Synthetic {}", register_index + 1), "CAD")?;
        for day in 1..=DAY_COUNT {
            let date = format!("2026-09-{day:02}");
            let session = database.open_session(register, &date, 1_000_000_000)?;
            let session_index = register_index * DAY_COUNT + day - 1;
            let session_entries =
                count / session_count + usize::from(session_index < count % session_count);
            let end = generated + session_entries;
            while generated < end {
                let batch_end = (generated + BATCH_SIZE).min(end);
                // Only one bounded batch owns strings; the full workload never lives in memory.
                let keys: Vec<String> = (generated..batch_end)
                    .map(|index| format!("synthetic-{index}"))
                    .collect();
                let entries: Vec<NewEntry<'_>> = keys
                    .iter()
                    .enumerate()
                    .map(|(offset, key)| {
                        let index = generated + offset;
                        let amount = 100 + (index % 10_000) as i64;
                        let direction = if index.is_multiple_of(5) {
                            Direction::Out
                        } else {
                            Direction::In
                        };
                        match direction {
                            Direction::In => expected_in += amount,
                            Direction::Out => expected_out += amount,
                        }
                        NewEntry {
                            direction,
                            amount,
                            reason: match index % 4 {
                                0 => "Supplier payment",
                                1 => "Retail sale",
                                2 => "Service revenue",
                                _ => "Cash adjustment",
                            },
                            request_key: key,
                        }
                    })
                    .collect();
                database.record_entries(session, &entries)?;
                generated = batch_end;
            }
            let balance = database.balance(session)?;
            database.close_session(session, balance.expected)?;
        }
    }
    let generation_elapsed = generation_started.elapsed();
    drop(database);
    let filter = DashboardFilter {
        start_date: "2026-09-01".into(),
        end_date: "2026-09-30".into(),
        currency: "CAD".into(),
        register_id: None,
    };
    println!(
        "Synthetic ledger: {count} entries, {REGISTER_COUNT} registers, {session_count} sessions"
    );
    println!(
        "Generation: {:.3} s ({:.0} entries/s)",
        generation_elapsed.as_secs_f64(),
        count as f64 / generation_elapsed.as_secs_f64()
    );
    println!("Expected totals (minor units): received={expected_in}, paid={expected_out}");
    for run in 1..=3 {
        let started = Instant::now();
        let snapshot = load_dashboard(&path, &filter)?;
        let elapsed = started.elapsed();
        verify_snapshot(&snapshot, count, expected_in, expected_out)?;
        println!(
            "Dashboard run {run}: {:.3} ms; counts and totals verified",
            elapsed.as_secs_f64() * 1_000.0
        );
    }
    // Closing connections before TempDir drops also removes SQLite's WAL sidecars.
    temporary.close()?;
    Ok(())
}

fn verify_snapshot(
    snapshot: &cash_counter::analytics::DashboardSnapshot,
    count: usize,
    expected_in: i64,
    expected_out: i64,
) -> Result<(), io::Error> {
    if snapshot.summary.movement_count != count as i64
        || snapshot.summary.sessions != (REGISTER_COUNT * DAY_COUNT) as i64
        || snapshot.summary.open_sessions != 0
        || snapshot.summary.closed_sessions != (REGISTER_COUNT * DAY_COUNT) as i64
        || snapshot.summary.cash_in != expected_in
        || snapshot.summary.cash_out != expected_out
        || snapshot.summary.net_flow != expected_in - expected_out
        || snapshot.summary.discrepancy_count != 0
        || snapshot.summary.discrepancy_total != 0
    {
        return Err(io::Error::other(
            "Dashboard counts or totals differ from the generated ledger",
        ));
    }
    Ok(())
}
