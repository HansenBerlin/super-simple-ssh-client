use std::fs;
use std::io::Write;
use std::path::Path;

use chrono::Datelike;

use crate::app::constants::{
    LOG_MAX_ENTRIES, LOG_MAX_IN_MEMORY, LOG_PARSE_FORMAT, LOG_RETENTION_DAYS,
    LOG_SEPARATOR, LOG_TIMESTAMP_FORMAT,
};
use crate::app::App;

impl App {
    pub(crate) fn set_status(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.status = message.clone();
        self.log_line(&message);
    }

    pub(super) fn log_line(&mut self, message: &str) {
        let timestamp = chrono::Local::now().format(LOG_TIMESTAMP_FORMAT);
        let line = format!("{timestamp}{LOG_SEPARATOR}{message}");
        if let Some(parent) = self.log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = writeln!(file, "{line}");
        }
        self.last_log = line.clone();
        self.log_lines.push_back(line);
        while self.log_lines.len() > LOG_MAX_IN_MEMORY {
            self.log_lines.pop_front();
        }
    }
}

pub(crate) fn prune_log_file(path: &Path) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let cutoff = chrono::Local::now().naive_local() - chrono::Duration::days(LOG_RETENTION_DAYS);
    let current_year = chrono::Local::now().year();
    let mut kept = Vec::new();
    for line in content.lines() {
        if let Some((timestamp, _)) = line.split_once(LOG_SEPARATOR) {
            let with_year = format!("{current_year}-{timestamp}");
            if let Ok(parsed) =
                chrono::NaiveDateTime::parse_from_str(&with_year, LOG_PARSE_FORMAT)
            {
                if parsed >= cutoff {
                    kept.push(line.to_string());
                }
            }
        }
    }
    if kept.len() > LOG_MAX_ENTRIES {
        kept = kept.split_off(kept.len().saturating_sub(LOG_MAX_ENTRIES));
    }
    if kept.is_empty() {
        let _ = fs::remove_file(path);
    } else if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
        let _ = fs::write(path, kept.join("\n") + "\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_log_path() -> std::path::PathBuf {
        let mut base = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        base.push(format!("ssh-client-log-test-{nanos}.log"));
        base
    }

    #[test]
    fn prune_log_file_removes_old_entries() {
        let path = temp_log_path();
        let now = chrono::Local::now().naive_local();
        let old = now - chrono::Duration::days(LOG_RETENTION_DAYS + 1);
        let recent = now - chrono::Duration::days(1);
        let old_line = format!("{}{}old", old.format(LOG_TIMESTAMP_FORMAT), LOG_SEPARATOR);
        let recent_line = format!("{}{}recent", recent.format(LOG_TIMESTAMP_FORMAT), LOG_SEPARATOR);
        fs::write(&path, format!("{old_line}\n{recent_line}\n")).unwrap();
        prune_log_file(&path);
        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("old"));
        assert!(content.contains("recent"));
    }
}
