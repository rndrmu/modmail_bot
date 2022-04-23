# Modmail

Discord bot for contacting a server's moderators easily and anonymously.

**Important Note:** This bot only works inside **one** server. It's not meant to be used as a globally hosted bot for multiple servers to use.

## Usage
### Compiling

* Clone the repository.

  ```bash
  git clone https://github.com/eficats/modmail
  ```

* Export `DATABASE_URL` (the path isn't important).
  ```bash
  export DATABASE_URL='sqlite:compiled.db'
  ```

* Compile with cargo.
  ```
  cargo build --release
  ```

### Running

* [Create a new application at Discord Developers if you haven't already.](https://discord.com/developers/applications)

* Create a `.env` file next to the executable with the following contents.
  ```sh
  # your discord API token
  DISCORD_TOKEN=

  # your discord API application ID
  DISCORD_APPID=

  # the ID to the server you'll be using the bot inside of
  DISCORD_GUILD=

  # (Optional) Set to change how verbose logging output is.
  # https://docs.rs/env_logger/latest/env_logger/#enabling-logging
  RUST_LOG=info
  ```

* Run the executable.

## License

The code in this repository is available under the [AGPLv3 License](https://www.gnu.org/licenses/agpl-3.0.en.html).
