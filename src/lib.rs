mod ai;
mod app;
mod cli;
mod commands;
mod commit;
mod config;
mod events;
mod git;
mod hooks;
mod risk;
mod store;

use anyhow::{Context, Result};
use clap::Parser;
use cli::Cli;
use std::env;

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    let cwd = env::current_dir().context("failed to read current directory")?;
    app::dispatch::run_cli(cli, cwd).await
}
