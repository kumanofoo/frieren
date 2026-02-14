use std::env;
use serenity::{
    async_trait,
    model::{
        channel::Message,
        gateway::Ready,
        id::ChannelId,
        application::Interaction,
        timestamp::Timestamp,
    },
    builder::{
        CreateEmbed,
        CreateMessage,
        CreateInteractionResponse,
        CreateInteractionResponseMessage,
    },
    prelude::*,
};

use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use chrono::{DateTime, Local, NaiveDate, Months, Timelike, TimeZone};
use serde::{Deserialize, Serialize};
use std::{fs::{self, OpenOptions}, path::PathBuf, sync::Arc};
use tokio::{
    time::{sleep, Duration},
    sync::broadcast,
    signal,
};
use log::{info, error, warn};
use anyhow::Result;

#[derive(Debug, Deserialize, Clone)]
struct Config {
    discord_token: String,
    database_path: String,
    storage_path: String,
    daily_post_time: String,
    post_channel_id: String,
}

impl Config {
    fn post_channel_id_u64(&self) -> u64 {
        match self.post_channel_id.parse() {
            Ok(p) => p,
            Err(e) => {
                warn!("post_channel_id parse error: {}", e);
                0
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct AttachmentInfo {
    original_name: String,
    original_url: String,
    image_path: String,
}

struct Handler {
    pool: SqlitePool,
    config: Arc<Config>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _: Context, ready: Ready) {
        info!("Logged in as {}", ready.user.name);
    }

    async fn message(&self, _: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        if let Err(e) = save_message(&self.pool, &self.config, &msg).await {
            error!("save_message error: {:?}", e);
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(cmd) = interaction {
            if cmd.data.name == "memory" {

                if let Err(e) = post_memory(&ctx.http, &self.pool, &self.config).await {
                    error!("memory command error: {:?}", e);
                }

                let _ = cmd.create_response(
                    &ctx.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content("Memory posted!")
                    )
                ).await;
            }
        }
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    if let Err(e) = run().await {
        error!("Fatal error: {:?}", e);
    }

    info!("Bot exited cleanly");
}

async fn run() -> Result<()> {
    let filename = env::var("FRIEREN_CONFIG").unwrap_or("config.toml".to_string());
    let config: Config = {
        let text = match fs::read_to_string(&filename) {
            Ok(t) => t,
            Err(e) => {
                error!("'{}': {}", filename, e);
                std::process::exit(1);
            }
        };
        toml::from_str(&text)?
    };
    config.post_channel_id.parse::<u64>()?;
    
    fs::create_dir_all(&config.storage_path)?;

    let db_path = PathBuf::from(&config.database_path);
    if let Some(parent) = db_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            error!("Failed to create DB directory: {:?}", e);
            return Err(e.into());
        }
    }

    if !db_path.exists() {
        info!("Database file not found. Creating new database file...");

        if let Err(e) = OpenOptions::new()
            .create(true)
            .write(true)
            .open(&db_path)
        {
            error!("Failed to create DB file: {:?}", e);
            return Err(e.into());
        }
    }

    let db_url = format!("sqlite:{}", db_path.display());
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    init_db(&pool).await?;

    info!("Database ready at {}", db_path.display());

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let handler = Handler {
        pool: pool.clone(),
        config: Arc::new(config.clone()),
    };

    let mut client = Client::builder(&config.discord_token, intents)
        .event_handler(handler)
        .await?;

    let http = client.http.clone();
    let pool_clone = pool.clone();
    let config_clone = Arc::new(config.clone());

    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    let mut shutdown_rx = shutdown_tx.subscribe();

    info!("Frieren starting...");
    tokio::spawn({
        let shutdown_tx = shutdown_tx.clone();
        async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Scheduler shutting down...");
                        break;
                    }
                    _ = sleep(Duration::from_secs(60)) => {
                        if let Err(e) = daily_scheduler(&http, &pool_clone, &config_clone).await {
                            error!("scheduler error: {:?}", e);
                        }
                    }
                }
            }
            drop(shutdown_tx);
        }
    });

    let shard_manager = client.shard_manager.clone();
    let client_handle = tokio::spawn(async move {
        if let Err(e) = client.start().await {
            error!("Client error: {:?}", e);
        }
    });

    wait_for_shutdown_signal().await;
    info!("Shutdown signal received");

    shard_manager.shutdown_all().await;

    let _ = shutdown_tx.send(());

    let _ = client_handle.await;

    info!("Graceful shutdown complete");

    Ok(())
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("Cannot install SIGTERM handler");

        tokio::select! {
            _ = signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        let _ = signal::ctrl_c().await;
    }
}

async fn init_db(pool: &SqlitePool) -> Result<()> {
    sqlx::query(include_str!("../schema.sql"))
        .execute(pool)
        .await?;
    Ok(())
}

async fn save_message(pool: &SqlitePool, config: &Config, msg: &Message) -> Result<()> {
    let timestamp = msg.timestamp.unix_timestamp();

    let mut attachments: Vec<AttachmentInfo> = Vec::new();
    for attachment in &msg.attachments {
        let ext = attachment.filename.split('.').last().unwrap_or("dat");
        let filename = format!("{}.{}", attachment.id, ext);
        let path = format!("{}/{}", config.storage_path, filename);

        match reqwest::get(&attachment.url).await {
            Ok(resp) => match resp.bytes().await {
                Ok(bytes) => {
                    if let Err(e) = fs::write(&path, bytes) {
                        warn!("Image write failed: {:?}", e);
                        continue;
                    }

                    attachments.push(AttachmentInfo {
                        original_name: attachment.filename.clone(),
                        original_url: attachment.url.clone(),
                        image_path: path,
                    });
                }
                Err(e) => warn!("Image bytes failed: {:?}", e),
            },
            Err(e) => warn!("Image download failed: {:?}", e),
        }
    }

    let attachments_json = match serde_json::to_string(&attachments) {
        Ok(json) => Some(json),
        Err(e) => {
            warn!("JSON serialize failed: {:?}", e);
            None
        }
    };

    if let Err(e) = sqlx::query(
        r#"
        INSERT INTO messages
        (message_id, channel_id, author_id, content,
         attachments_json,
         created_at)
        VALUES (?, ?, ?, ?, ?, ?)
        "#
    )
    .bind(msg.id.to_string())
    .bind(msg.channel_id.to_string())
    .bind(msg.author.id.to_string())
    .bind(&msg.content)
    .bind(attachments_json)
    .bind(timestamp)
    .execute(pool)
    .await {
        error!("DB insert failed: {:?}", e);
    }

    Ok(())
}

async fn daily_scheduler(
    http: &serenity::http::Http,
    pool: &SqlitePool,
    config: &Config,
) -> Result<()> {

    let now = Local::now();
    let current_time = format!("{:02}:{:02}", now.hour(), now.minute());

    if current_time == config.daily_post_time {
        if let Err(e) = post_memory(http, pool, config).await {
            error!("daily post failed: {:?}", e);
        }
    }

    Ok(())
}

#[derive(Debug)]
struct MemoryRow {
    content: Option<String>,
    attachments_json: Option<String>,
    created_at: String,
}

async fn fetch_by_range(
    pool: &SqlitePool,
    date: NaiveDate,
) -> Result<Vec<MemoryRow>> {
    let start = date.and_hms_opt(0, 0, 0)
        .and_then(|dt| Local.from_local_datetime(&dt).single())
        .map(|dt| dt.timestamp());

    let end = date.and_hms_opt(23, 59, 59)
        .and_then(|dt| Local.from_local_datetime(&dt).single())
        .map(|dt| dt.timestamp());

    let (start, end) = match (start, end) {
        (Some(s), Some(e)) => (s, e),
        _ => {
            warn!("Failed to calculate day range for {}", date);
            return Ok(vec![]);
        }
    };

    let rows = sqlx::query_as!(
        MemoryRow,
        r#"
        SELECT content,
               attachments_json,
               created_at
        FROM messages
        WHERE created_at >= ? AND created_at <= ?
        ORDER BY created_at DESC
        "#,
        start,
        end
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

async fn fetch_past_today(
    pool: &SqlitePool,
    today: NaiveDate,
) -> Result<Vec<MemoryRow>> {

    let start = today.and_hms_opt(0, 0, 0)
        .and_then(|dt| Local.from_local_datetime(&dt).single())
        .map(|dt| dt.timestamp());

    let end = today.and_hms_opt(23, 59, 59)
        .and_then(|dt| Local.from_local_datetime(&dt).single())
        .map(|dt| dt.timestamp());

    let (start, end) = match (start, end) {
        (Some(s), Some(e)) => (s, e),
        _ => return Ok(vec![]),
    };

    let rows = sqlx::query_as!(
        MemoryRow,
        r#"
        SELECT content,
               attachments_json,
               created_at
        FROM messages
        WHERE created_at >= ?
          AND created_at <= ?
          AND strftime('%Y', datetime(created_at, 'unixepoch'))
              < strftime('%Y', 'now', 'localtime')
        ORDER BY created_at DESC
        "#,
        start,
        end
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

async fn fetch_yesterday(pool: &SqlitePool) -> Result<Vec<MemoryRow>> {
    let yesterday = Local::now().date_naive() - chrono::Duration::days(1);
    fetch_by_range(pool, yesterday).await
}

async fn fetch_one_week_ago(pool: &SqlitePool) -> Result<Vec<MemoryRow>> {
    let date = Local::now().date_naive() - chrono::Duration::days(7);
    fetch_by_range(pool, date).await
}

fn one_month_ago_safe(now: DateTime<Local>) -> NaiveDate {
    now.checked_sub_months(Months::new(1))
        .map(|d| d.date_naive())
        .unwrap_or_else(|| {
            log::warn!("checked_sub_months failed, fallback 30 days");
            (now - chrono::Duration::days(30)).date_naive()
        })
}

async fn fetch_one_month_ago(pool: &SqlitePool) -> Result<Vec<MemoryRow>> {
    let date = one_month_ago_safe(Local::now());
    fetch_by_range(pool, date).await
}

async fn post_memory(
    http: &serenity::http::Http,
    pool: &SqlitePool,
    config: &Config,
) -> Result<()> {
    let today = Local::now().date_naive();

    let mut rows: Vec<MemoryRow> = Vec::new();
    let mut source_label = "";

    match fetch_past_today(pool, today).await {
        Ok(r) if !r.is_empty() => {
            rows = r;
            source_label = "📅 Past Memory";
        }
        Ok(_) => {},
        Err(e) => error!("fetch_past_today error: {:?}", e),
    }

    if rows.is_empty() {
        match fetch_one_month_ago(pool).await {
            Ok(r) if !r.is_empty() => {
                rows = r;
                source_label = "📆 One Month Ago";
            }
            Ok(_) => {},
            Err(e) => error!("fetch_one_month_ago error: {:?}", e),
        }
    }
    
    if rows.is_empty() {
        match fetch_one_week_ago(pool).await {
            Ok(r) if !r.is_empty() => {
                rows = r;
                source_label = "🗓 One Week Ago";
            }
            Ok(_) => {},
            Err(e) => error!("fetch_one_week_ago error: {:?}", e),
        }
    }
    
    if rows.is_empty() {
        match fetch_yesterday(pool).await {
            Ok(r) if !r.is_empty() => {
                rows = r;
                source_label = "⏪ Yesterday";
            }
            Ok(_) => {},
            Err(e) => error!("fetch_yesterday error: {:?}", e),
        }
    }
    
    if rows.is_empty() {
        warn!("No memory found.");
        return Ok(());
    }
    
    for row in rows {
        let mut embed = CreateEmbed::new()
            .title(source_label)
            .description(row.content.unwrap_or_default());

        match row.created_at.parse() {
            Ok(t) => match Timestamp::from_unix_timestamp(t) {
                Ok(t) => embed = embed.timestamp(t),
                Err(e) => warn!("Failed to parse timestamp: {:?}", e),
            },
            Err(e) => warn!("Failed to parse timestamp: {:?}", e),
        }

        if let Some(json) = row.attachments_json {
            if let Ok(list) = serde_json::from_str::<Vec<AttachmentInfo>>(&json) {
                if let Some(first) = list.first() {
                    embed = embed.image(first.original_url.clone());
                }

                for att in list.iter().skip(1) {
                    embed = embed.field(
                        "Image",
                        format!("[{}]({})", att.original_name, att.original_url),
                        false,
                    );
                }
            }
        }

        let message = CreateMessage::new().embed(embed);
 
        if let Err(e) = ChannelId::new(config.post_channel_id_u64())
            .send_message(http, message)
            .await
        {
            error!("Discord send failed: {:?}", e);
        }
    }

    info!("Memory post completed successfully.");
    Ok(())
}
