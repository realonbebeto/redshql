use tracing::dispatcher::set_global_default;
use tracing_bunyan_formatter::{BunyanFormattingLayer, JsonStorageLayer};
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};

pub mod handler;
pub mod load;
pub mod s3;
pub mod statements;

pub fn init_tracing_subscriber() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug"));

    let formatting_layer = BunyanFormattingLayer::new("redshql".into(), std::io::stdout);

    let registry = Registry::default()
        .with(env_filter)
        .with(JsonStorageLayer)
        .with(formatting_layer);

    set_global_default(registry.into()).expect("Failed to set subscriber");
}
