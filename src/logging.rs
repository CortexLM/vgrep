use std::path::PathBuf;
use std::str::FromStr;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{fmt, EnvFilter, Layer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub fn init_logging(log_level: Option<String>, log_file: Option<PathBuf>) {
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level.unwrap_or_else(|| "info".to_string())))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let stdout_layer = fmt::layer()
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .with_level(false)
        .compact()
        .with_filter(env_filter.clone());

    let registry = tracing_subscriber::registry();

    if let Some(path) = log_file {
        let file_appender = tracing_appender::rolling::never(
            path.parent().unwrap_or(&PathBuf::from(".")),
            path.file_name().unwrap_or(&std::ffi::OsString::from("vgrep.log")),
        );
        let file_layer = fmt::layer()
            .with_ansi(false)
            .with_file(true)
            .with_line_number(true)
            .with_thread_ids(true)
            .with_target(true)
            .with_span_events(FmtSpan::CLOSE)
            .with_writer(file_appender)
            .with_filter(env_filter);
            
        registry.with(stdout_layer).with(file_layer).init();
    } else {
        registry.with(stdout_layer).init();
    }
}
