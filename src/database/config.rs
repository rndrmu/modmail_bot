use std::{
    fmt::{self, Debug, Display},
    str::FromStr,
};

use crate::error::Result;
use serenity::model::id::{ChannelId, RoleId};
use sqlx::SqlitePool;

pub trait ConfigKey: Display {
    type Value: Display + FromStr;
}

pub struct Config(SqlitePool);

impl Config {
    pub fn new(pool: SqlitePool) -> Self {
        Self(pool)
    }

    pub async fn get<T>(&self, key: T) -> Result<Option<T::Value>>
    where
        T: ConfigKey,
        <<T as ConfigKey>::Value as FromStr>::Err: Debug,
    {
        let key = &key.to_string();
        Ok(sqlx::query!("SELECT value FROM config WHERE key = ?", key)
            .fetch_optional(&self.0)
            .await
            .map_err(anyhow::Error::from)?
            .map(|r| {
                let value = &r.value;
                T::Value::from_str(value).expect("got malformed config from database")
            }))
    }

    pub async fn set<T>(&self, key: T, value: T::Value) -> Result<()>
    where
        T: ConfigKey,
    {
        let (key, value) = (&key.to_string(), &value.to_string());
        let res = sqlx::query!(
            "INSERT INTO config (key, value) VALUES (?, ?)
            ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            key,
            value
        )
        .execute(&self.0)
        .await
        .map_err(anyhow::Error::from)?;

        assert_eq!(res.rows_affected(), 1u64);
        Ok(())
    }

    pub async fn unset<T>(&self, key: T) -> Result<()>
    where
        T: ConfigKey,
    {
        let key = &key.to_string();
        sqlx::query!("DELETE FROM config WHERE key = ?", key)
            .execute(&self.0)
            .await
            .map_err(anyhow::Error::from)?;
        Ok(())
    }
}

pub struct Blockrole;

impl Display for Blockrole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "blockrole")
    }
}

impl ConfigKey for Blockrole {
    type Value = RoleId;
}

pub struct Inbox;

impl Display for Inbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "inbox")
    }
}

impl ConfigKey for Inbox {
    type Value = ChannelId;
}

#[cfg(test)]
mod tests {
    use serenity::model::id::{ChannelId, RoleId};
    use sqlx::SqlitePool;

    use super::{Blockrole, Config, Inbox};

    #[tokio::test]
    async fn config_crud() {
        // Setup
        let config = {
            let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
            sqlx::migrate!().run(&pool).await.unwrap();
            Config::new(pool)
        };

        // Create
        config.set(Blockrole, RoleId(123)).await.unwrap();
        config.set(Inbox, ChannelId(456)).await.unwrap();

        // Get
        let blockrole = config.get(Blockrole).await.unwrap().unwrap();
        let inbox = config.get(Inbox).await.unwrap().unwrap();
        assert_eq!(blockrole, RoleId(123));
        assert_eq!(inbox, ChannelId(456));

        // Update
        config.set(Blockrole, RoleId(321)).await.unwrap();
        config.set(Inbox, ChannelId(654)).await.unwrap();

        // Get
        let blockrole = config.get(Blockrole).await.unwrap().unwrap();
        let inbox = config.get(Inbox).await.unwrap().unwrap();
        assert_eq!(blockrole, RoleId(321));
        assert_eq!(inbox, ChannelId(654));

        // Delete
        config.unset(Blockrole).await.unwrap();
        config.unset(Inbox).await.unwrap();

        // Get
        let blockrole = config.get(Blockrole).await.unwrap();
        let inbox = config.get(Inbox).await.unwrap();
        assert_eq!(blockrole, None);
        assert_eq!(inbox, None);
    }
}
