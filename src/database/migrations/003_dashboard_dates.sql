-- Date-range analytics and reports should not scan unrelated session history.
CREATE INDEX register_sessions_business_date ON register_sessions(business_date, register_id);
