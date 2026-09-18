//! Perception — high-throughput real-time object detection, face recognition, and OCR.

mod capture;
mod config;
mod download;
mod engine;
mod error;
mod face_db;
mod pipeline;
mod preview;
mod storage;
mod types;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "perception")]
#[command(about = "Real-time object detection, face recognition, and OCR pipeline")]
#[command(version)]
struct Cli {
    /// Path to configuration file.
    #[arg(short, long, global = true, default_value = "perception.toml")]
    config: PathBuf,

    /// Increase log verbosity (-v, -vv, -vvv).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the perception pipeline.
    Run {
        /// Record annotated output to an MP4 file.
        #[arg(long)]
        record: Option<PathBuf>,
    },
    /// Download or update models.
    Download,
    /// Manage known faces database.
    Faces {
        #[command(subcommand)]
        action: FacesAction,
    },
    /// Show system info (GPU, execution providers, loaded models).
    Info,
}

#[derive(Subcommand)]
enum FacesAction {
    /// Add a known face from an image.
    Add { name: String, image: PathBuf },
    /// List all known faces.
    List,
    /// Remove a known face by name.
    Remove { name: String },
}

fn init_tracing(verbosity: u8) {
    let filter = match verbosity {
        0 => "perception=info",
        1 => "perception=debug",
        2 => "perception=trace",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        Commands::Run { record } => {
            let config = config::Config::load(&cli.config)?;
            tracing::info!(
                source = %config.capture.source,
                path = %config.capture.path,
                "starting perception pipeline"
            );

            // Ensure models are available.
            let model_paths = download::ensure_models(&config).await?;

            // Build pipeline and run.
            pipeline::run(config, model_paths, record).await?;
        }
        Commands::Download => {
            let config = config::Config::load(&cli.config)?;
            download::ensure_models(&config).await?;
            tracing::info!("all models downloaded");
        }
        Commands::Faces { action } => {
            let config = config::Config::load(&cli.config)?;
            match action {
                FacesAction::Add { name, image } => {
                    face_db::add_face(&config, &name, &image).await?;
                    tracing::info!(%name, "face added");
                }
                FacesAction::List => {
                    let faces = face_db::list_faces(&config)?;
                    if faces.is_empty() {
                        println!("No known faces.");
                    } else {
                        for f in &faces {
                            println!("  {f}");
                        }
                    }
                }
                FacesAction::Remove { name } => {
                    face_db::remove_face(&config, &name)?;
                    tracing::info!(%name, "face removed");
                }
            }
        }
        Commands::Info => {
            println!("Perception v{}", env!("CARGO_PKG_VERSION"));
            engine::print_info();
        }
    }

    Ok(())
}
