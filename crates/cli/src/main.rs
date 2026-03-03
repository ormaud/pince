//! `pince` CLI binary.
//!
//! Current subcommands:
//!   pince secret set <name>    — read value from stdin, store secret
//!   pince secret list          — print secret names
//!   pince secret delete <name> — remove a secret
//!   pince secret show <name>   — print secret value (human use only)

use std::{
    io::{self, Read},
    path::PathBuf,
};

use anyhow::{bail, Result};
use secrets::SecretStore;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("secret") => handle_secret(&args[2..]),
        Some(other) => bail!("unknown subcommand: {other}\nUsage: pince secret <set|list|delete|show>"),
        None => {
            eprintln!("Usage: pince <subcommand>");
            eprintln!("  pince secret <set|list|delete|show>");
            Ok(())
        }
    }
}

fn handle_secret(args: &[String]) -> Result<()> {
    let store = open_store()?;
    match args.first().map(|s| s.as_str()) {
        Some("set") => {
            let name = args.get(1).ok_or_else(|| anyhow::anyhow!("Usage: pince secret set <name>"))?;
            let mut value = String::new();
            io::stdin().read_to_string(&mut value)?;
            let value = value.trim_end_matches('\n');
            store.set(name, value.as_bytes())?;
            eprintln!("Secret '{name}' stored.");
        }
        Some("list") => {
            let names = store.list()?;
            if names.is_empty() {
                println!("(no secrets stored)");
            } else {
                for name in names {
                    println!("{name}");
                }
            }
        }
        Some("delete") => {
            let name = args.get(1).ok_or_else(|| anyhow::anyhow!("Usage: pince secret delete <name>"))?;
            store.delete(name)?;
            eprintln!("Secret '{name}' deleted.");
        }
        Some("show") => {
            let name = args.get(1).ok_or_else(|| anyhow::anyhow!("Usage: pince secret show <name>"))?;
            eprintln!("Warning: displaying secret value for '{name}'.");
            let val = store.resolve(name)?;
            let s = val.expose_str().unwrap_or("<binary value>");
            println!("{s}");
        }
        Some(other) => bail!("unknown secret subcommand: {other}\nUsage: pince secret <set|list|delete|show>"),
        None => bail!("Usage: pince secret <set|list|delete|show>"),
    }
    Ok(())
}

fn open_store() -> Result<SecretStore> {
    let dir = secrets_dir()?;
    SecretStore::new(dir)
}

fn secrets_dir() -> Result<PathBuf> {
    if let Ok(val) = std::env::var("PINCE_SECRETS_DIR") {
        return Ok(PathBuf::from(val));
    }
    let config = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".config")
        });
    Ok(config.join("pince").join("secrets"))
}
