mod agent;
mod app;
mod clipboard;
mod cli;
mod config;
mod git;
mod notify;
mod pane;
mod restore;
mod tmux;
mod update;
mod wizard;

fn main() -> anyhow::Result<()> {
    cli::run()
}
