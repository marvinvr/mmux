mod agent;
mod agentmgr;
mod app;
mod cli;
mod clipboard;
mod config;
mod git;
mod notify;
mod open;
mod pane;
mod restore;
mod tmux;
mod update;
mod wizard;
mod workspacemgr;

fn main() -> anyhow::Result<()> {
    cli::run()
}
