mod api;
mod auth;
mod cli;
mod help;
mod input;
mod question;
mod runner;

use anyhow::{Result, ensure};
use clap::Parser;
use cli::{Cli, Command};
use std::io::IsTerminal;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let command = Cli::parse().command;
    if let Command::Auth { command } = command {
        return auth::run(command);
    }
    let (options, question) = command.evaluation()?;
    ensure!(
        !std::io::stdin().is_terminal(),
        "supply state through stdin (pipe or redirect a file)"
    );
    let api = api::Api::new(&options, auth::key()?)?;
    let states = input::states(tokio::io::BufReader::new(tokio::io::stdin()), options.input);
    runner::run(
        states,
        tokio::io::stdout(),
        options.concurrency.into(),
        |_, state| api.evaluate(state, &question),
    )
    .await
}
