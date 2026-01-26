pub(crate) const STATUS_READY: &str = "Ready";
pub(crate) const STATUS_CANCELLED: &str = "Cancelled";

pub(crate) const LOG_TIMESTAMP_FORMAT: &str = "%m-%d %H:%M:%S";
pub(crate) const LOG_PARSE_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
pub(crate) const LOG_SEPARATOR: &str = " | ";
pub(crate) const LOG_NO_LOGS_MESSAGE: &str = "No logs yet";

pub(crate) const LOG_RETENTION_DAYS: i64 = 7;
pub(crate) const LOG_MAX_ENTRIES: usize = 10_000;
pub(crate) const LOG_MAX_IN_MEMORY: usize = 100;

pub(crate) const NOT_CONNECTED_MESSAGE: &str = "Selected connection is not connected";

pub(crate) const TRANSFER_LOG_THRESHOLD_BYTES: u64 = 1024 * 1024;
pub(crate) const MB_BYTES: f64 = 1024.0 * 1024.0;

pub(crate) const NOTICE_NOT_CONNECTED_TITLE: &str = "Not connected";
pub(crate) const NOTICE_NOT_CONNECTED_MESSAGE: &str = "Please connect to the host machine first.";
pub(crate) const NOTICE_NO_SUBFOLDERS_TITLE: &str = "No subfolders";
pub(crate) const NOTICE_NO_SUBFOLDERS_MESSAGE: &str =
    "This folder has no subfolders. To select it as the target, press S.";
