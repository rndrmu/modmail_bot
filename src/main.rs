use std::time::Duration;

use anyhow::Context;
use modmail::Bot;
use serenity::{client::ClientBuilder, prelude::GatewayIntents};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

const INTENTS: GatewayIntents = GatewayIntents::from_bits_truncate(
    GatewayIntents::DIRECT_MESSAGES.bits()
        | GatewayIntents::GUILD_MESSAGES.bits()
        | GatewayIntents::GUILDS.bits()
        | GatewayIntents::MESSAGE_CONTENT.bits(),
);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt().init();
    let token = std::env::var("DISCORD_TOKEN").context("DISCORD_TOKEN missing")?;
    let appid: u64 = std::env::var("DISCORD_APPID")
        .context("DISCORD_APPID missing")?
        .parse()
        .context("DISCORD_APPID is not a valid ID")?;

    let guild: u64 = std::env::var("DISCORD_GUILD")
        .context("DISCORD_GUILD missing")?
        .parse()
        .context("DISCORD_GUILD is not a valid ID")?;

    let pool = {
        let opts = SqliteConnectOptions::new()
            .create_if_missing(true)
            .filename("bot.db")
            .journal_mode(SqliteJournalMode::Wal);
        SqlitePoolOptions::new()
            .max_lifetime(Duration::from_secs(3600))
            .max_connections(2)
            .connect_with(opts)
            .await
            .context("failed to connect to DB")?
    };

    sqlx::migrate!()
        .run(&pool)
        .await
        .context("failed to migrate")?;

    let bot = Bot::new(pool.clone(), guild);
    let mut client = ClientBuilder::new(token, INTENTS)
        .application_id(appid)
        .event_handler(bot)
        .await
        .context("failed to build client")?;

    tokio::select! {
        res = client.start() => res.context("failed to start client")?,
        _ = tokio::signal::ctrl_c() => {
            tracing::warn!("caught interrupt signal (Ctrl-C), exiting...");
        },
    };

    pool.close().await;
    Ok(())
}
