//! For the isolated real-app harness: the app's keyring entry copied over a pipe into another
//! session's keyring, never through a file. `get` prints it (run in the real session), `set`
//! reads it from stdin (run inside the isolated one).
use anyhow::{anyhow, Result};
use std::io::{Read, Write};

fn main() -> Result<()> {
    let entry = keyring::Entry::new("dev.quince.client", "session")?;
    match std::env::args().nth(1).as_deref() {
        Some("get") => std::io::stdout().write_all(entry.get_password()?.as_bytes())?,
        Some("set") => {
            let mut secret = String::new();
            std::io::stdin().read_to_string(&mut secret)?;
            entry.set_password(&secret)?;
        }
        _ => return Err(anyhow!("get or set")),
    }
    Ok(())
}
