//! Discord notification service
//!
//! Listens to NATS events (discord.message, system.alert) and forwards
//! them to a Discord channel via Serenity bot.

use futures::StreamExt;
use vm_nats::subjects::discord as discord_events;
use vm_nats::subjects::system as system_events;
use serenity::all::ChannelId;
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;
use std::env;
use std::error::Error;

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, _ctx: Context, _msg: Message) {}

    async fn ready(&self, _: Context, ready: Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);
    }
}

/// Start Discord notification service
///
/// Spawns two tasks:
/// 1. Discord client (Serenity)
/// 2. NATS subscriber that forwards messages to Discord
pub async fn start_discord_service(
    nats_client: async_nats::Client,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let token = match env::var("DISCORD_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            tracing::error!("DISCORD_TOKEN not found, skipping Discord service");
            return Ok(());
        }
    };

    let channel_id = env::var("DISCORD_CHANNEL_ID")
        .unwrap_or_else(|_| "1400608400227827953".to_string())
        .parse::<u64>()
        .expect("Invalid DISCORD_CHANNEL_ID");

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let mut client = Client::builder(&token, intents)
        .event_handler(Handler)
        .await
        .expect("Failed to create Discord client");

    let http = client.http.clone();

    // Spawn NATS subscriber task
    tokio::task::spawn(async move {
        let mut sub_discord = nats_client
            .queue_subscribe(discord_events::MESSAGE, "discord-notifier".to_string())
            .await
            .expect("Failed to subscribe to discord message");

        let mut sub_system_alert = nats_client
            .queue_subscribe(system_events::ALERT, "discord-notifier".to_string())
            .await
            .expect("Failed to subscribe to system alert");

        let channel = ChannelId::new(channel_id);

        tracing::info!("Discord service connected to NATS, listening for events...");

        loop {
            tokio::select! {
                msg = sub_discord.next() => {
                    match msg {
                        Some(msg) => {
                            if let Ok(text) = String::from_utf8(msg.payload.to_vec()) {
                                tracing::info!("Sending message to Discord: {}", text);
                                if let Err(e) = channel.say(http.clone(), &text).await {
                                    tracing::error!("Failed to send Discord message: {}", e);
                                }
                            }
                        }
                        None => {
                            tracing::warn!("[NATS] Discord message subscription ended, resubscribing in 1s...");
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            match nats_client.queue_subscribe(discord_events::MESSAGE, "discord-notifier".to_string()).await {
                                Ok(new_sub) => sub_discord = new_sub,
                                Err(e) => tracing::error!("[NATS] Failed to resubscribe to discord message: {e}"),
                            }
                        }
                    }
                }
                msg = sub_system_alert.next() => {
                    match msg {
                        Some(msg) => {
                            if let Ok(text) = String::from_utf8(msg.payload.to_vec()) {
                                tracing::info!("Sending system alert to Discord: {}", text);
                                if let Err(e) = channel.say(http.clone(), &text).await {
                                    tracing::error!("Failed to send Discord system alert: {}", e);
                                }
                            }
                        }
                        None => {
                            tracing::warn!("[NATS] Discord system alert subscription ended, resubscribing in 1s...");
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            match nats_client.queue_subscribe(system_events::ALERT, "discord-notifier".to_string()).await {
                                Ok(new_sub) => sub_system_alert = new_sub,
                                Err(e) => tracing::error!("[NATS] Failed to resubscribe to system alert: {e}"),
                            }
                        }
                    }
                }
            }
        }
    });

    // Start Discord client (blocks until shutdown)
    tokio::task::spawn(async move {
        if let Err(why) = client.start().await {
            tracing::error!("Discord client error: {:?}", why);
        }
    });

    tracing::info!("Discord notification service started");
    Ok(())
}
