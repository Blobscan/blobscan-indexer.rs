use tracing::{subscriber::set_global_default, Subscriber};
use tracing_log::LogTracer;
use tracing_subscriber::{
    fmt::{self, MakeWriter},
    prelude::__tracing_subscriber_SubscriberExt,
    EnvFilter, Registry,
};

/// Gets a subscriber that can be used to initialize the logger.
pub fn get_subscriber<Sink>(env_filter: String, sink: Sink) -> impl Subscriber + Send + Sync
where
    Sink: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(env_filter));
    let formatting_layer = fmt::layer()
        .compact() // Use the Pretty formatter.
        .with_writer(sink);

    Registry::default()
        .with(env_filter)
        .with(formatting_layer)
        .with(sentry_tracing::layer())
}

/// Inits the logger with the given subscriber.
pub fn init_subscriber(subscriber: impl Subscriber + Send + Sync) {
    LogTracer::init().expect("Could not initialize formatting layer");

    set_global_default(subscriber).expect("Failed to set subscriber");
}
