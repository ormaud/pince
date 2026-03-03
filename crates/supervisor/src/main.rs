use anyhow::Result;
use supervisor_lib::{config::Config, supervisor::Supervisor};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Config::from_env();
    tracing::info!("starting pince supervisor");

    let sup = Supervisor::new(config).await?;
    sup.run().await
}
