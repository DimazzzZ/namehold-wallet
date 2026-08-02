-- Cross-process sync coordination for background daemon + app manual sync.
-- Each profile can only be synced by one process at a time.
-- The lock holder refreshes a heartbeat; stale locks (heartbeat > 30s old) are taken over.
CREATE TABLE sync_locks (
  profile_id TEXT PRIMARY KEY,
  owner_pid INTEGER NOT NULL,
  owner_type TEXT NOT NULL CHECK(owner_type IN ('app', 'daemon')),
  acquired_at INTEGER NOT NULL,  -- unix timestamp (seconds)
  heartbeat_at INTEGER NOT NULL  -- unix timestamp (seconds), refreshed by holder
);

-- Index for stale lock queries (heartbeat_at < now - 30s).
CREATE INDEX idx_sync_locks_heartbeat ON sync_locks(heartbeat_at);
