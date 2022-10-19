use std::io;

use clap::Parser;
use tracing::debug;

#[derive(Debug, clap::Parser)]
struct Args {
    /// The command to run
    #[arg(last(true), num_args(1..), required(true))]
    cmd: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .pretty()
        .with_file(false)
        .without_time()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let args = Args::parse();
    debug!(?args);

    Ok(())
}
