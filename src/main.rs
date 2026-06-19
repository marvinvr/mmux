mod app;
mod clipboard;
mod cli;
mod config;
mod git;
mod notify;
mod pane;
mod tmux;
mod wizard;

fn main() -> anyhow::Result<()> {
    cli::run()
}
