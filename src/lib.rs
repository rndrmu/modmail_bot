use std::{num::ParseIntError, result::Result as StdResult};

use serenity::{
    async_trait,
    client::{Context, EventHandler},
    model::{
        channel::{ChannelType, PartialChannel},
        gateway::Ready,
        guild::Role,
        id::{ChannelId, GuildId, RoleId, UserId},
        interactions::application_command::{ApplicationCommandOptionType, ApplicationCommandType},
    },
};
use sqlx::{FromRow, SqlitePool};

#[derive(thiserror::Error, Debug)]
enum BotError {
    #[error("{0}")]
    UserError(String),
    #[error("You sent an unknown command. Please contact the developer.")]
    UnknownCommand(String),
    #[error("There was an error processing your command.")]
    InternalError(#[from] anyhow::Error),
}

type Result<T> = StdResult<T, BotError>;

pub struct Bot {
    guild: GuildId,
    pool: SqlitePool,
}

impl Bot {
    pub fn new<T>(pool: SqlitePool, guild: T) -> Self
    where
        T: Into<GuildId>,
    {
        Self {
            pool,
            guild: guild.into(),
        }
    }

    async fn config(&self, key: &str) -> Result<Option<String>> {
        Ok(sqlx::query!("SELECT value FROM config WHERE key = ?", key)
            .fetch_optional(&self.pool)
            .await
            .map_err(anyhow::Error::from)?
            .map(|r| r.value))
    }

    async fn set_config(&self, key: &str, value: &str) -> Result<()> {
        let res = sqlx::query!(
            "INSERT INTO config (key, value) VALUES (?, ?)
            ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            key,
            value
        )
        .execute(&self.pool)
        .await
        .map_err(anyhow::Error::from)?;

        assert_eq!(res.rows_affected(), 1u64);
        Ok(())
    }

    async fn unset_config(&self, key: &str) -> Result<()> {
        let res = sqlx::query!("DELETE FROM config WHERE key = ?", key)
            .execute(&self.pool)
            .await
            .map_err(anyhow::Error::from)?;

        assert_eq!(res.rows_affected(), 1u64);
        Ok(())
    }

    async fn get_blockrole(&self) -> Result<Option<RoleId>> {
        let raw = self.config("blockrole").await?;
        Ok(raw.map(|s| RoleId(s.parse().expect("got malformed ID from database"))))
    }

    async fn set_blockrole(&self, role: &Role) -> Result<()> {
        let id = role.id.0.to_string();
        self.set_config("blockrole", &id).await
    }

    async fn unset_blockrole(&self) -> Result<()> {
        self.unset_config("blockrole").await
    }

    async fn get_inbox(&self) -> Result<Option<ChannelId>> {
        let raw = self.config("inbox").await?;
        Ok(raw.map(|s| ChannelId(s.parse().expect("got malformed ID from database"))))
    }

    async fn set_inbox(&self, channel: &PartialChannel) -> Result<()> {
        let id = channel.id.0.to_string();
        self.set_config("inbox", &id).await
    }

    async fn unset_inbox(&self) -> Result<()> {
        self.unset_config("inbox").await
    }

    async fn find_codename(&self, codename: &str) -> Result<Option<Room>> {
        Ok(
            sqlx::query_as!(RawRoom, "SELECT * FROM rooms WHERE codename = ?", codename)
                .fetch_optional(&self.pool)
                .await
                .map_err(anyhow::Error::from)?
                .map(|rt| Room::try_from(rt).expect("got malformed thread from database")),
        )
    }

    async fn find_channel(&self, id: u64) -> Result<Option<Room>> {
        let temp = &id.to_string();
        Ok(
            sqlx::query_as!(RawRoom, "SELECT * FROM rooms WHERE channel_id = ?", temp)
                .fetch_optional(&self.pool)
                .await
                .map_err(anyhow::Error::from)?
                .map(|rt| Room::try_from(rt).expect("got malformed thread from database")),
        )
    }

    async fn find_user(&self, id: u64) -> Result<Option<Room>> {
        let temp = &id.to_string();
        Ok(
            sqlx::query_as!(RawRoom, "SELECT * FROM rooms WHERE user_id = ?", temp)
                .fetch_optional(&self.pool)
                .await
                .map_err(anyhow::Error::from)?
                .map(|rt| Room::try_from(rt).expect("got malformed thread from database")),
        )
    }
}

#[async_trait]
impl EventHandler for Bot {
    async fn ready(&self, ctx: Context, _: Ready) {
        self.guild
            .set_application_commands(&ctx, |cmds| {
                cmds.create_application_command(|cmd| {
                    cmd.name("block")
                        .description("Block a user from using the bot.")
                        .kind(ApplicationCommandType::ChatInput)
                        .create_option(|opt| {
                            opt.name("codename")
                                .description("The codename. Must be an exact match.")
                                .kind(ApplicationCommandOptionType::String)
                                .required(true)
                        })
                })
                .create_application_command(|cmd| {
                    cmd.name("blockrole")
                        .description("Manage the role given to blocked users.")
                        .kind(ApplicationCommandType::ChatInput)
                        .create_option(|opt| {
                            opt.name("set")
                                .description("Set the role given to blocked users.")
                                .kind(ApplicationCommandOptionType::SubCommand)
                                .create_sub_option(|sub| {
                                    sub.name("role")
                                        .description("The role to be used.")
                                        .kind(ApplicationCommandOptionType::Role)
                                        .required(true)
                                })
                        })
                        .create_option(|opt| {
                            opt.name("unset")
                                .description("Unset the block role.")
                                .kind(ApplicationCommandOptionType::SubCommand)
                        })
                })
                .create_application_command(|cmd| {
                    cmd.name("inbox")
                        .description("Manage the channel threads will be added to.")
                        .kind(ApplicationCommandType::ChatInput)
                        .create_option(|opt| {
                            opt.name("set")
                                .description("Set the channel threads will be added to.")
                                .kind(ApplicationCommandOptionType::SubCommand)
                                .create_sub_option(|sub| {
                                    sub.name("channel")
                                        .description("The channel to be used. Must allow threads.")
                                        .kind(ApplicationCommandOptionType::Channel)
                                        .channel_types(&[ChannelType::Text])
                                        .required(true)
                                })
                        })
                        .create_option(|opt| {
                            opt.name("unset")
                                .description("Unset the inbox channel.")
                                .kind(ApplicationCommandOptionType::SubCommand)
                        })
                })
                .create_application_command(|cmd| {
                    cmd.name("close")
                        .description("Close this thread and forget the attached user.")
                        .kind(ApplicationCommandType::ChatInput)
                        .create_option(|opt| {
                            opt.name("codename")
                                .description("The codename. Must be an exact match.")
                                .kind(ApplicationCommandOptionType::String)
                                .required(true)
                        })
                })
            })
            .await
            .expect("failed to register commands");
    }
}

#[derive(FromRow)]
struct RawRoom {
    room_id: i64,
    codename: String,
    channel_id: String,
    user_id: String,
}

struct Room {
    room_id: i64,
    codename: String,
    channel_id: ChannelId,
    user_id: UserId,
}

impl TryFrom<RawRoom> for Room {
    type Error = ParseIntError;

    fn try_from(value: RawRoom) -> StdResult<Self, Self::Error> {
        Ok(Self {
            room_id: value.room_id,
            codename: value.codename,
            channel_id: value.channel_id.parse::<u64>()?.into(),
            user_id: value.user_id.parse::<u64>()?.into(),
        })
    }
}
