use std::{num::ParseIntError, result::Result as StdResult};

use serenity::{
    async_trait,
    client::{Context, EventHandler},
    model::{
        channel::{ChannelType, Message, PartialChannel, PartialGuildChannel},
        gateway::Ready,
        guild::Role,
        id::{ChannelId, GuildId, RoleId, UserId},
        interactions::{
            application_command::{
                ApplicationCommandInteraction,
                ApplicationCommandInteractionDataOptionValue as OptionValue,
                ApplicationCommandOptionType, ApplicationCommandType,
            },
            Interaction,
        },
    },
    prelude::Mentionable,
    utils::{Color, MessageBuilder},
};
use sqlx::{FromRow, SqlitePool};

#[derive(thiserror::Error, Debug)]
enum BotError {
    #[error("{0}")]
    UserError(String),
    #[error(
        "You sent an unimplemented command. Please file an issue: {}/issues",
        env!("CARGO_PKG_REPOSITORY")
    )]
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
        sqlx::query!("DELETE FROM config WHERE key = ?", key)
            .execute(&self.pool)
            .await
            .map_err(anyhow::Error::from)?;
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

    async fn room_from_codename(&self, codename: &str) -> Result<Option<Room>> {
        Ok(
            sqlx::query_as!(RawRoom, "SELECT * FROM rooms WHERE codename = ?", codename)
                .fetch_optional(&self.pool)
                .await
                .map_err(anyhow::Error::from)?
                .map(|rt| Room::try_from(rt).expect("got malformed thread from database")),
        )
    }

    async fn room_from_channel(&self, channel_id: u64) -> Result<Option<Room>> {
        let temp = &channel_id.to_string();
        Ok(
            sqlx::query_as!(RawRoom, "SELECT * FROM rooms WHERE channel_id = ?", temp)
                .fetch_optional(&self.pool)
                .await
                .map_err(anyhow::Error::from)?
                .map(|rt| Room::try_from(rt).expect("got malformed thread from database")),
        )
    }

    async fn room_from_user(&self, user_id: u64) -> Result<Option<Room>> {
        let temp = &user_id.to_string();
        Ok(
            sqlx::query_as!(RawRoom, "SELECT * FROM rooms WHERE user_id = ?", temp)
                .fetch_optional(&self.pool)
                .await
                .map_err(anyhow::Error::from)?
                .map(|rt| Room::try_from(rt).expect("got malformed thread from database")),
        )
    }

    async fn delete_room(&self, room_id: i64) -> Result<()> {
        sqlx::query!("DELETE FROM rooms WHERE room_id = ?", room_id)
            .execute(&self.pool)
            .await
            .map_err(anyhow::Error::from)?;
        Ok(())
    }

    async fn new_room(&self, codename: &str, channel_id: u64, user_id: u64) -> Result<()> {
        // HACK: query!() drops temporaries for some reason, must pass reference
        let (channel_id, user_id) = (&channel_id.to_string(), &user_id.to_string());
        let res = sqlx::query!(
            "INSERT INTO rooms (codename, channel_id, user_id) VALUES (?, ?, ?)",
            codename,
            channel_id,
            user_id
        )
        .execute(&self.pool)
        .await
        .map_err(anyhow::Error::from)?;

        assert_eq!(res.rows_affected(), 1);
        Ok(())
    }

    async fn check_codename_exists(&self, codename: &str) -> Result<bool> {
        // HACK: macro doesn't work, treats EXISTS() as a column name
        let (exists,): (bool,) =
            sqlx::query_as("SELECT EXISTS(SELECT 1 FROM rooms WHERE codename = ?)")
                .bind(codename)
                .fetch_one(&self.pool)
                .await
                .map_err(anyhow::Error::from)?;
        Ok(exists)
    }

    async fn execute_command(
        &self,
        ctx: &Context,
        cmd: &ApplicationCommandInteraction,
    ) -> Result<String> {
        let perms = cmd.member.as_ref().unwrap().permissions.unwrap();
        match cmd.data.name.as_str() {
            "blockrole" => {
                if !perms.manage_roles() {
                    return Err(BotError::UserError(
                        "You don't have `Manage Roles` permission.".into(),
                    ));
                }

                let sub = cmd.data.options.get(0).unwrap();
                match sub.name.as_str() {
                    "set" => {
                        let role = sub.options.get(0).unwrap().resolved.as_ref().unwrap();
                        if let OptionValue::Role(role) = role {
                            self.set_blockrole(role).await?;
                            Ok(format!("Set block role to `{}`.", role.name.as_str()))
                        } else {
                            panic!("got wrong option value")
                        }
                    }

                    "unset" => {
                        self.unset_blockrole().await?;
                        Ok("Unset block role.".into())
                    }

                    _ => Err(BotError::UnknownCommand(format!(
                        "{} {}",
                        &cmd.data.name, &sub.name
                    ))),
                }
            }

            "inbox" => {
                if !perms.manage_channels() {
                    return Err(BotError::UserError(
                        "You don't have `Manage Channels` permission.".into(),
                    ));
                }

                let sub = cmd.data.options.get(0).unwrap();
                match sub.name.as_str() {
                    "set" => {
                        let raw = sub.options.get(0).unwrap().resolved.as_ref().unwrap();
                        if let OptionValue::Channel(channel) = raw {
                            self.set_inbox(channel).await?;
                            Ok(format!("Set inbox to {}.", channel.id.mention()))
                        } else {
                            panic!("got wrong option value")
                        }
                    }

                    "unset" => {
                        self.unset_inbox().await?;
                        Ok("Unset inbox.".into())
                    }

                    _ => Err(BotError::UnknownCommand(format!(
                        "{} {}",
                        &cmd.data.name, &sub.name
                    ))),
                }
            }

            "block" => {
                if !perms.manage_roles() {
                    return Err(BotError::UserError(
                        "You don't have `Manage Roles` permission.".into(),
                    ));
                }

                let role = self.get_blockrole().await.and_then(|opt| {
                    opt.ok_or_else(|| BotError::UserError("There's no block role defined.".into()))
                })?;

                let codename = cmd.data.options.get(0).unwrap().resolved.as_ref().unwrap();
                if let OptionValue::String(codename) = codename {
                    let room = self.room_from_codename(codename).await.and_then(|opt| {
                        opt.ok_or_else(|| {
                            BotError::UserError(format!(
                                "No thread with codename `{}` found.",
                                codename
                            ))
                        })
                    })?;

                    let mut member = self.guild.member(ctx, room.user_id).await.map_err(|_| {
                        BotError::UserError(
                            "User is not a member or the server is unavailable.".into(),
                        )
                    })?;

                    member.add_role(ctx, role).await.map_err(|_| {
                        BotError::UserError(
                            "Missing permissions or configured block role is invalid.".into(),
                        )
                    })?;

                    Ok(format!("Blocked `{}`.", &codename))
                } else {
                    panic!("got wrong option value")
                }
            }

            "close" => {
                if !perms.manage_channels() {
                    return Err(BotError::UserError(
                        "You don't have `Manage Channels` permission.".into(),
                    ));
                }

                let codename = cmd.data.options.get(0).unwrap().resolved.as_ref().unwrap();
                if let OptionValue::String(codename) = codename {
                    let room = self.room_from_codename(codename).await.and_then(|opt| {
                        opt.ok_or_else(|| {
                            BotError::UserError(format!(
                                "No thread with codename `{}` found.",
                                codename
                            ))
                        })
                    })?;

                    let _ = room
                        .channel_id
                        .edit_thread(ctx, |edit| edit.archived(true))
                        .await;

                    self.delete_room(room.room_id).await?;
                    Ok(format!(
                        "Archived `{}` and removed attached user.",
                        &codename
                    ))
                } else {
                    panic!("got wrong option value")
                }
            }

            _ => Err(BotError::UnknownCommand(cmd.data.name.clone())),
        }
    }

    async fn handle_message(&self, ctx: &Context, msg: &Message) -> Result<Option<String>> {
        if msg.author.id == ctx.cache.current_user().id {
            return Ok(None);
        }

        if msg.is_private() {
            match self.get_blockrole().await? {
                Some(role) => {
                    let blocked = msg
                        .author
                        .has_role(ctx, self.guild, role)
                        .await
                        .map_err(anyhow::Error::from)?;

                    if blocked {
                        return Ok(Some("You have been blocked by a server admin.".into()));
                    }
                }

                None => (),
            }

            if let Some(room) = self.room_from_user(msg.author.id.0).await? {
                let content = MessageBuilder::new().push_safe(&msg.content).build();
                room.channel_id
                    .send_message(ctx, |createmsg| createmsg.content(content))
                    .await
                    .map_err(anyhow::Error::from)?;

                Ok(None)
            } else {
                let inbox = match self.get_inbox().await? {
                    Some(inbox) => inbox,
                    None => return Ok(None),
                };

                let codename = loop {
                    let candidate = petname::petname(2, " ");
                    if !self.check_codename_exists(&candidate).await? {
                        break candidate;
                    }
                };

                let thread = {
                    let inbox_msg = inbox
                        .say(ctx, "Got mail from new user.")
                        .await
                        .map_err(anyhow::Error::from)?;

                    inbox
                        .create_public_thread(ctx, inbox_msg.id, |thread| thread.name(&codename))
                        .await
                        .map_err(anyhow::Error::from)?
                };

                let content = MessageBuilder::new().push_safe(&msg.content).build();
                thread
                    .send_message(ctx, |createmsg| createmsg.content(content))
                    .await
                    .map_err(anyhow::Error::from)?;

                self.new_room(&codename, thread.id.0, msg.author.id.0)
                    .await?;

                Ok(Some(format!(
                    "You've been assigned the codename `{}`.",
                    &codename
                )))
            }
        } else {
            let room = match self.room_from_channel(msg.channel_id.0).await? {
                Some(room) => room,
                None => return Ok(None),
            };

            let content = MessageBuilder::new().push_safe(&msg.content).build();
            room.user_id
                .create_dm_channel(ctx)
                .await
                .map_err(anyhow::Error::from)?
                .send_message(ctx, |msg| msg.content(content))
                .await
                .map_err(anyhow::Error::from)?;

            Ok(None)
        }
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

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Some(cmd) = interaction.application_command() {
            assert_eq!(cmd.guild_id.unwrap(), self.guild);
            let res = self.execute_command(&ctx, &cmd).await;
            let (color, desc) = match res {
                Ok(msg) => (Color::DARK_GREEN, msg),
                Err(err) => {
                    if let BotError::InternalError(ref err) = err {
                        tracing::error!(source = ?err, "Error while handling interaction.");
                    }
                    (Color::DARK_RED, err.to_string())
                }
            };

            cmd.create_interaction_response(&ctx, |res| {
                res.interaction_response_data(|data| {
                    data.embed(|emb| {
                        emb.description(desc)
                            .color(color)
                            .footer(|foot| foot.text("With \u{2764} from the post office."))
                    })
                })
            })
            .await
            .expect("failed to send interaction response");
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        let res = self.handle_message(&ctx, &msg).await;
        match res {
            Ok(content) => {
                if let Some(content) = content {
                    msg.channel_id
                        .send_message(&ctx, |send| {
                            send.reference_message(&msg).embed(|emb| {
                                emb.color(Color::BLURPLE)
                                    .description(content)
                                    .footer(|foot| foot.text("With \u{2764} from the post office."))
                            })
                        })
                        .await
                        .expect("failed to send interaction response");
                }
            }
            Err(err) => {
                if let BotError::InternalError(err) = err {
                    tracing::error!(source = ?err, "Error while handling message.");
                }
            }
        }
    }

    async fn thread_delete(&self, _: Context, thread: PartialGuildChannel) {
        let res = match self.room_from_channel(thread.id.0).await {
            Ok(opt) => {
                if let Some(room) = opt {
                    self.delete_room(room.room_id).await
                } else {
                    return;
                }
            }
            Err(e) => Err(e),
        };

        if let Err(e) = res {
            tracing::error!(source = ?e, "Error while handling thread deletion.");
        }
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
