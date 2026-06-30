-- Re-introduce a configurable hsd data directory ("prefix") so the app can
-- start hsd against a user-chosen location (e.g. an external volume like
-- /Volumes/WD/hsd-data) instead of dumping a large chain into ~/.hsd.
-- Empty value = fall back to hsd's own default (~/.hsd).
INSERT OR IGNORE INTO settings (key, value) VALUES ('hsd_prefix', '');
