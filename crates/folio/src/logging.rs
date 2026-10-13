//! Logging and crash reporting.
//!
//! Logs go to stderr and to a rolling file, so a bug report can be traced
//! without asking the reader to reproduce it under a terminal. A panic is
//! logged with its payload and location before the process gives up.

use std::path::PathBuf;

use tracing_subscriber::EnvFilter;

/// Start logging. Returns the guard that must stay alive for the file writer,
/// and the log file's directory for display in error messages.
pub fn init() -> (
    Option<tracing_appender::non_blocking::WorkerGuard>,
    Option<PathBuf>,
) {
    let filter = EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new("folio=info,folio_core=info,warn"));

    let log_dir = directories::ProjectDirs::from("", "", "folio")
        .map(|dirs| dirs.data_local_dir().join("logs"));

    let (guard, dir) = match log_dir {
        Some(dir) if std::fs::create_dir_all(&dir).is_ok() => {
            let appender = tracing_appender::rolling::daily(&dir, "folio.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let result = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(writer)
                .with_ansi(false)
                .try_init();
            if result.is_err() {
                eprintln!("folio: logging was already initialized");
            }
            (Some(guard), Some(dir))
        }
        _ => {
            let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
            (None, None)
        }
    };

    install_panic_hook(dir.clone());
    (guard, dir)
}

fn install_panic_hook(log_dir: Option<PathBuf>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".into());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        tracing::error!(%location, %payload, "folio panicked");
        if let Some(dir) = &log_dir {
            eprintln!("folio: crashed — see the log in {}", dir.display());
        }
        previous(info);
    }));
}
