# Frieren

**Frieren** is a Discord bot written in Rust that automatically **backs up messages and reposts past memories**.
It stores messages and attachments in a SQLite database and can repost messages from the past (e.g., the same day in previous years, one month ago, etc.).

## Features

* Save all messages posted in a Discord channel
* Download and archive attachments (images/files)
* Store message metadata in **SQLite**
* Daily automatic reposting of past memories
* Slash command `/memory` to manually trigger a memory post
* Smart fallback logic when no past memory exists:

  1. Same date in previous years
  2. One month ago
  3. One week ago
  4. Yesterday

## Architecture

```text
Discord
   │
   ▼
Frieren Bot (Rust / Serenity)
   │
   ├─ SQLite database (messages.db)
   │
   └─ Attachment storage (images/)
```

Messages are stored in SQLite and attachments are downloaded locally for backup.

## Requirements

* Rust 1.75+
* SQLite
* systemd (for service deployment)
* Discord Bot Token

## Installation

### Build from Source

```bash
git clone https://github.com/yourrepo/frieren.git
cd frieren

cargo build --release
```

The binary will be located at:

```text
target/release/frieren
```

### Debian Package

This project supports building `.deb` packages using **cargo-deb**.

Install cargo-deb:

```bash
cargo install cargo-deb
```

Build package:

```bash
cargo deb
```

The package will be generated in:

```text
target/debian/
```

### Configuration

Create a configuration file:

```text
/etc/frieren/config.toml
```

Example configuration:

```toml
discord_token = "YOUR_DISCORD_BOT_TOKEN"
database_path = "/var/lib/frieren/messages.db"
storage_path = "/var/lib/frieren/images"
daily_post_time = "07:00"
post_channel_id = "123456789012345678"
```

### Configuration Fields

| Field           | Description                                  |
| --------------- | -------------------------------------------- |
| discord_token   | Discord bot token                            |
| database_path   | SQLite database path                         |
| storage_path    | Directory where attachments are stored       |
| daily_post_time | Time of day to automatically repost memories |
| post_channel_id | Channel where memory posts will appear       |

### Database

The bot uses a simple SQLite schema.

Table: `messages`

| Column           | Description                   |
| ---------------- | ----------------------------- |
| message_id       | Discord message ID            |
| channel_id       | Discord channel ID            |
| author_id        | User ID                       |
| content          | Message text                  |
| attachments_json | JSON metadata for attachments |
| created_at       | Unix timestamp                |

Attachments are stored separately on disk.

## Running the Bot

Set the config path using an environment variable:

```bash
export FRIEREN_CONFIG=/etc/frieren/config.toml
./frieren
```

By default, Frieren reads the configuration file from: `config.toml`
in the current working directory.

### Slash Command

```text
/memory
```

Forces the bot to post a memory immediately.

### Automatic Memory Posting

At the configured time (`daily_post_time`), the bot attempts to repost memories in the following order:

1. Same date in previous years
2. One month ago
3. One week ago
4. Yesterday

If no messages are found, nothing is posted.

## systemd Service

Example service file:

```text
/etc/systemd/system/frieren.service
```

```ini
[Unit]
Description=Frieren Discord Memory Bot
After=network.target

[Service]
User=frieren
Environment=FRIEREN_CONFIG=/etc/frieren/config.toml
ExecStart=/usr/bin/frieren
Restart=always

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable frieren
sudo systemctl start frieren
```

## Logging

Logging is handled with `env_logger`.

Example:

```shell
RUST_LOG=info frieren
```

Log levels supported:

```console
error
warn
info
debug
trace
```

## License

[MIT License](LICENSE "MIT License")
