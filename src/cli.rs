use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Clone, Parser)]
#[command(name = "hodoq", version, about)]
pub struct Cli {
    /// Use an alternative data directory.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
}
