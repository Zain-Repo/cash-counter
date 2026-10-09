-- Supports newest-first keyset paging without sorting a register's full history.
CREATE INDEX register_sessions_history ON register_sessions(register_id, id);
