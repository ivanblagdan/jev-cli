use crate::cli::Auth;
use anyhow::{Context, Result, ensure};
use inquire::{Password, PasswordDisplayMode};

fn credential() -> Result<keyring::Entry> {
    Ok(keyring::Entry::new("jev.typesafe.ai", "api-key")?)
}

pub fn key() -> Result<String> {
    let key = match std::env::var("TYPESAFE_API_KEY") {
        Ok(key) => key,
        Err(std::env::VarError::NotPresent) => credential()?
            .get_password()
            .context("no API key; set TYPESAFE_API_KEY or run `jev auth set-key`")?,
        Err(error) => return Err(error.into()),
    };
    ensure!(!key.trim().is_empty(), "API key is empty");
    Ok(key)
}

pub fn run(command: Auth) -> Result<()> {
    match command {
        Auth::SetKey => {
            let key = Password::new("TypeSafe API key:")
                .with_display_mode(PasswordDisplayMode::Masked)
                .without_confirmation()
                .prompt()?;
            ensure!(!key.trim().is_empty(), "API key must not be empty");
            credential()?.set_password(&key)?;
            eprintln!("API key saved in the OS credential store.");
        }
        Auth::DeleteKey => {
            credential()?.delete_credential()?;
            eprintln!("API key removed.");
        }
    }
    Ok(())
}
