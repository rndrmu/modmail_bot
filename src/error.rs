use std::result::Result as StdResult;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    User(String),
    #[error(
        "You sent an unimplemented command. Please file an issue: {}/issues",
        env!("CARGO_PKG_REPOSITORY")
    )]
    UnknownCommand(String),
    #[error("There was an error processing your command.")]
    Internal(#[from] anyhow::Error),
}

pub type Result<T> = StdResult<T, Error>;
