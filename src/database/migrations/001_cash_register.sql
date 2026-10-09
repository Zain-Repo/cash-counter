-- Version 1: immutable cash movements and transactionally maintained session totals.
CREATE TABLE registers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK(length(trim(name)) BETWEEN 1 AND 100),
    currency TEXT NOT NULL CHECK(length(currency) = 3 AND currency NOT GLOB '*[^A-Z]*'),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE register_sessions (
    id INTEGER PRIMARY KEY,
    register_id INTEGER NOT NULL REFERENCES registers(id),
    business_date TEXT NOT NULL CHECK(length(business_date) = 10 AND date(business_date, '+0 days') IS business_date),
    opening INTEGER NOT NULL CHECK(opening >= 0),
    cash_in_total INTEGER NOT NULL DEFAULT 0 CHECK(cash_in_total >= 0),
    cash_out_total INTEGER NOT NULL DEFAULT 0 CHECK(cash_out_total >= 0),
    expected INTEGER GENERATED ALWAYS AS (opening + cash_in_total - cash_out_total) STORED
        CHECK(typeof(expected) = 'integer'),
    counted INTEGER CHECK(counted >= 0),
    difference INTEGER GENERATED ALWAYS AS (counted - expected) STORED
        CHECK(difference IS NULL OR typeof(difference) = 'integer'),
    opened_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    closed_at TEXT,
    CHECK((counted IS NULL) = (closed_at IS NULL)),
    UNIQUE(register_id, business_date)
) STRICT;
CREATE UNIQUE INDEX one_open_session_per_register ON register_sessions(register_id) WHERE closed_at IS NULL;

-- Opening balances and closed counts are historical facts.
CREATE TRIGGER session_identity_immutable BEFORE UPDATE OF register_id, business_date, opening, opened_at ON register_sessions
BEGIN SELECT RAISE(ABORT, 'Session identity is immutable'); END;
CREATE TRIGGER closed_session_immutable BEFORE UPDATE ON register_sessions WHEN OLD.closed_at IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'Session is closed'); END;
CREATE TRIGGER session_no_delete BEFORE DELETE ON register_sessions
BEGIN SELECT RAISE(ABORT, 'Session history is immutable'); END;

CREATE TABLE cash_in (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES register_sessions(id),
    amount INTEGER NOT NULL CHECK(amount > 0),
    reason TEXT NOT NULL CHECK(length(trim(reason)) BETWEEN 1 AND 500),
    request_key TEXT NOT NULL UNIQUE CHECK(length(trim(request_key)) BETWEEN 1 AND 128),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;
CREATE INDEX cash_in_session_id ON cash_in(session_id, id);
CREATE TRIGGER cash_in_open_session BEFORE INSERT ON cash_in
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM register_sessions WHERE id = NEW.session_id AND closed_at IS NULL
    ) THEN RAISE(ABORT, 'An open session is required') END;
END;
-- The entry and its summary update succeed or roll back together, including overflow.
CREATE TRIGGER cash_in_total AFTER INSERT ON cash_in
BEGIN
    UPDATE register_sessions SET cash_in_total = cash_in_total + NEW.amount
    WHERE id = NEW.session_id;
END;
CREATE TRIGGER cash_in_no_update BEFORE UPDATE ON cash_in
BEGIN SELECT RAISE(ABORT, 'Cash entries are immutable'); END;
CREATE TRIGGER cash_in_no_delete BEFORE DELETE ON cash_in
BEGIN SELECT RAISE(ABORT, 'Cash entries are immutable'); END;

CREATE TABLE cash_out (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES register_sessions(id),
    amount INTEGER NOT NULL CHECK(amount > 0),
    reason TEXT NOT NULL CHECK(length(trim(reason)) BETWEEN 1 AND 500),
    request_key TEXT NOT NULL UNIQUE CHECK(length(trim(request_key)) BETWEEN 1 AND 128),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;
CREATE INDEX cash_out_session_id ON cash_out(session_id, id);
CREATE TRIGGER cash_out_open_session BEFORE INSERT ON cash_out
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM register_sessions WHERE id = NEW.session_id AND closed_at IS NULL
    ) THEN RAISE(ABORT, 'An open session is required') END;
END;
-- The entry and its summary update succeed or roll back together, including overflow.
CREATE TRIGGER cash_out_total AFTER INSERT ON cash_out
BEGIN
    UPDATE register_sessions SET cash_out_total = cash_out_total + NEW.amount
    WHERE id = NEW.session_id;
END;
CREATE TRIGGER cash_out_no_update BEFORE UPDATE ON cash_out
BEGIN SELECT RAISE(ABORT, 'Cash entries are immutable'); END;
CREATE TRIGGER cash_out_no_delete BEFORE DELETE ON cash_out
BEGIN SELECT RAISE(ABORT, 'Cash entries are immutable'); END;
