use std::{
    fs::{File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use anyhow::{Result, anyhow};
use chrono::Local;

const MAX_LOG_VALUE_CHARS: usize = 64_000;

struct Diagnostics {
    file: Mutex<File>,
    directory: PathBuf,
    path: PathBuf,
}

static DIAGNOSTICS: OnceLock<Diagnostics> = OnceLock::new();

/// 初始化按日期保存的 JSON Lines 日志文件。
pub fn init(data_dir: &Path) -> Result<PathBuf> {
    if let Some(diagnostics) = DIAGNOSTICS.get() {
        return Ok(diagnostics.path.clone());
    }
    let directory = data_dir.join("logs");
    std::fs::create_dir_all(&directory)?;
    let filename = format!("ruiz-{}.log", Local::now().format("%Y-%m-%d"));
    let path = directory.join(filename);
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    DIAGNOSTICS
        .set(Diagnostics {
            file: Mutex::new(file),
            directory,
            path: path.clone(),
        })
        .map_err(|_| anyhow!("诊断日志已经初始化"))?;
    info(
        "app.session.start",
        "Ruiz started",
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "log_path": path.display().to_string(),
        }),
    );
    Ok(path)
}

pub fn info(event: &str, message: &str, fields: serde_json::Value) {
    write("INFO", event, message, fields);
}

pub fn warn(event: &str, message: &str, fields: serde_json::Value) {
    write("WARN", event, message, fields);
}

pub fn error(event: &str, message: &str, fields: serde_json::Value) {
    write("ERROR", event, message, fields);
}

pub fn log_path() -> Option<PathBuf> {
    DIAGNOSTICS
        .get()
        .map(|diagnostics| diagnostics.path.clone())
}

pub fn log_hint() -> String {
    log_path()
        .map(|path| format!("诊断日志：{}", path.display()))
        .unwrap_or_else(|| "诊断日志未能初始化".into())
}

pub fn open_log_directory() -> Result<()> {
    let directory = DIAGNOSTICS
        .get()
        .map(|diagnostics| diagnostics.directory.clone())
        .ok_or_else(|| anyhow!("诊断日志尚未初始化"))?;
    open::that_detached(&directory)
        .map_err(|error| anyhow!("无法打开日志目录 {}: {error}", directory.display()))
}

pub fn truncate(value: &str) -> String {
    let mut truncated = value.chars().take(MAX_LOG_VALUE_CHARS).collect::<String>();
    if value.chars().count() > MAX_LOG_VALUE_CHARS {
        truncated.push_str("\n...[truncated]");
    }
    truncated
}

fn write(level: &str, event: &str, message: &str, fields: serde_json::Value) {
    let entry = serde_json::json!({
        "timestamp": Local::now().to_rfc3339(),
        "level": level,
        "event": event,
        "message": message,
        "fields": fields,
    });
    let Some(diagnostics) = DIAGNOSTICS.get() else {
        eprintln!("{entry}");
        return;
    };
    match diagnostics.file.lock() {
        Ok(mut file) => {
            if writeln!(file, "{entry}").is_err() || file.flush().is_err() {
                eprintln!("failed to write Ruiz diagnostics: {entry}");
            }
        }
        Err(_) => eprintln!("Ruiz diagnostics lock poisoned: {entry}"),
    }
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncates_large_log_values_on_character_boundaries() {
        let input = "好".repeat(64_001);
        let output = truncate(&input);
        assert!(output.ends_with("...[truncated]"));
        assert!(output.starts_with('好'));
    }
}
