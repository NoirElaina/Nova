use std::fs;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use tauri::{AppHandle, Manager};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

static FILE_LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();
static FILE_LOG_ENABLED: OnceLock<AtomicBool> = OnceLock::new();

fn file_log_enabled_flag() -> &'static AtomicBool {
    FILE_LOG_ENABLED.get_or_init(|| AtomicBool::new(true))
}

pub fn set_file_logging_enabled(enabled: bool) {
    file_log_enabled_flag().store(enabled, Ordering::Relaxed);
}

pub fn is_file_logging_enabled() -> bool {
    file_log_enabled_flag().load(Ordering::Relaxed)
}

#[derive(Clone)]
struct ToggleFileWriter<W> {
    inner: W,
}

enum ToggleFileWriteHandle<A, B> {
    Enabled(A),
    Disabled(B),
}

impl<'a, W> MakeWriter<'a> for ToggleFileWriter<W>
where
    W: MakeWriter<'a> + Clone,
{
    type Writer = ToggleFileWriteHandle<W::Writer, io::Sink>;

    fn make_writer(&'a self) -> Self::Writer {
        if is_file_logging_enabled() {
            ToggleFileWriteHandle::Enabled(self.inner.make_writer())
        } else {
            ToggleFileWriteHandle::Disabled(io::sink())
        }
    }
}

impl<A, B> Write for ToggleFileWriteHandle<A, B>
where
    A: Write,
    B: Write,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            ToggleFileWriteHandle::Enabled(writer) => writer.write(buf),
            ToggleFileWriteHandle::Disabled(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            ToggleFileWriteHandle::Enabled(writer) => writer.flush(),
            ToggleFileWriteHandle::Disabled(writer) => writer.flush(),
        }
    }
}

fn build_env_filter() -> EnvFilter {
    if let Ok(value) = std::env::var("NOVA_LOG") {
        return EnvFilter::new(value);
    }

    if let Ok(value) = std::env::var("RUST_LOG") {
        return EnvFilter::new(value);
    }

    EnvFilter::new("info,nova_lib=debug")
}

fn load_initial_file_logging_enabled(app: &AppHandle) -> bool {
    let Ok(app_data_dir) = app.path().app_data_dir() else {
        return true;
    };
    let settings_path = app_data_dir.join("settings.json");
    let Ok(content) = std::fs::read_to_string(settings_path) else {
        return true;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return true;
    };

    json.get("enableAppLog")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

fn install_panic_hook() {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let location = panic_info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown".to_string());

        let payload = if let Some(message) = panic_info.payload().downcast_ref::<&str>() {
            (*message).to_string()
        } else if let Some(message) = panic_info.payload().downcast_ref::<String>() {
            message.clone()
        } else {
            "non-string panic payload".to_string()
        };

        tracing::error!(
            location = %location,
            message = %payload,
            "application panic"
        );

        previous_hook(panic_info);
    }));
}

pub fn init(app: &AppHandle) -> Result<(), String> {
    set_file_logging_enabled(load_initial_file_logging_enabled(app));
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Failed to resolve app_data_dir for logs: {}", error))?;
    let log_dir = app_data_dir.join("logs");
    fs::create_dir_all(&log_dir).map_err(|error| {
        format!(
            "Failed to create log directory {}: {}",
            log_dir.display(),
            error
        )
    })?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "app.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let _ = FILE_LOG_GUARD.set(guard);
    let toggle_file_writer = ToggleFileWriter { inner: file_writer };

    let filter = build_env_filter();
    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(toggle_file_writer);
    let console_layer = fmt::layer().compact().with_target(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .try_init()
        .map_err(|error| format!("Failed to initialize tracing subscriber: {}", error))?;

    install_panic_hook();
    tracing::info!(
        log_dir = %log_dir.display(),
        file_logging_enabled = is_file_logging_enabled(),
        "application logging initialized"
    );
    Ok(())
}
