use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use nova_protocol::redact::format_panic_summary;
use tauri::{AppHandle, Manager};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

static FILE_LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();
static FILE_LOG_ENABLED: OnceLock<AtomicBool> = OnceLock::new();

fn file_log_enabled_flag() -> &'static AtomicBool {
    FILE_LOG_ENABLED.get_or_init(|| AtomicBool::new(false))
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
        return false;
    };
    let settings_path = app_data_dir.join("settings.json");
    let Ok(content) = std::fs::read_to_string(settings_path) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };

    json.get("enableAppLog")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn install_panic_hook(log_dir: PathBuf) {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let (location, summary) = format_panic_summary(panic_info, 2048);

        tracing::error!(
            location = %location,
            message = %summary,
            "application panic"
        );

        let crash_path = log_dir.join("crash.log");
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&crash_path)
        {
            let _ = writeln!(
                file,
                "[{}] location={} message={}",
                chrono::Utc::now().to_rfc3339(),
                location,
                summary
            );
        }

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

    install_panic_hook(log_dir.clone());
    tracing::info!(
        log_dir = %log_dir.display(),
        file_logging_enabled = is_file_logging_enabled(),
        "application logging initialized"
    );
    Ok(())
}
