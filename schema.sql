PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    message_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    author_id  TEXT NOT NULL,

    content TEXT,
    attachments_json TEXT,

    created_at TEXT NOT NULL

);

CREATE INDEX IF NOT EXISTS idx_messages_created_at
ON messages(created_at);
