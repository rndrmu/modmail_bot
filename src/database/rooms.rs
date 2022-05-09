use crate::error::Result;
use serenity::model::id::{ChannelId, UserId};
use sqlx::{FromRow, SqlitePool};
use std::{num::ParseIntError, result::Result as StdResult};

pub struct Room {
    pub room_id: i64,
    pub codename: String,
    pub channel_id: ChannelId,
    pub user_id: UserId,
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

impl Room {
    pub async fn new(
        pool: &SqlitePool,
        codename: String,
        channel_id: ChannelId,
        user_id: UserId,
    ) -> Result<Self> {
        // HACK: query!() drops temporaries for some reason, must pass reference
        let (channel_str, user_str) = (&channel_id.to_string(), &user_id.to_string());
        let room_id = sqlx::query!(
            "INSERT INTO rooms (codename, channel_id, user_id) VALUES (?, ?, ?) RETURNING room_id",
            codename,
            channel_str,
            user_str
        )
        .fetch_one(pool)
        .await
        .map_err(anyhow::Error::from)?
        .room_id;

        Ok(Self {
            room_id,
            codename,
            channel_id,
            user_id,
        })
    }

    pub async fn get_by_codename(pool: &SqlitePool, codename: &str) -> Result<Option<Self>> {
        Ok(
            sqlx::query_as!(RawRoom, "SELECT * FROM rooms WHERE codename = ?", codename)
                .fetch_optional(pool)
                .await
                .map_err(anyhow::Error::from)?
                .map(|rt| Room::try_from(rt).expect("got malformed Room object from database")),
        )
    }

    pub async fn get_by_channel(pool: &SqlitePool, channel_id: ChannelId) -> Result<Option<Self>> {
        // HACK: query!() drops temporaries for some reason, must pass reference
        let temp = &channel_id.to_string();
        Ok(
            sqlx::query_as!(RawRoom, "SELECT * FROM rooms WHERE channel_id = ?", temp)
                .fetch_optional(pool)
                .await
                .map_err(anyhow::Error::from)?
                .map(|rt| Room::try_from(rt).expect("got malformed Room object from database")),
        )
    }

    pub async fn get_by_user(pool: &SqlitePool, user_id: UserId) -> Result<Option<Self>> {
        // HACK: query!() drops temporaries for some reason, must pass reference
        let temp = &user_id.to_string();
        Ok(
            sqlx::query_as!(RawRoom, "SELECT * FROM rooms WHERE user_id = ?", temp)
                .fetch_optional(pool)
                .await
                .map_err(anyhow::Error::from)?
                .map(|rt| Room::try_from(rt).expect("got malformed Room object from database")),
        )
    }

    pub async fn delete(self, pool: &SqlitePool) -> Result<()> {
        sqlx::query!("DELETE FROM rooms WHERE room_id = ?", self.room_id)
            .execute(pool)
            .await
            .map_err(anyhow::Error::from)?;
        Ok(())
    }
}

#[derive(FromRow)]
struct RawRoom {
    room_id: i64,
    codename: String,
    channel_id: String,
    user_id: String,
}
