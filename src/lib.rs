pub mod cli;
pub mod config;
pub mod health;
pub mod service;

pub const BIN_NAME: &str = "unspace-dock";
pub const SYSTEMD_UNIT_NAME: &str = "unspace-dock";
pub const BIN_INSTALL_PATH: &str = "/usr/local/bin/unspace-dock";

pub const CONFIG_VERSION: u32 = 1;
pub const DEFAULT_CONFIG_PATH: &str = "/etc/unspace/config.json";
pub const PID_FILE: &str = "/run/unspace/unspace-dock.pid";
pub const RUNTIME_DIR: &str = "/run/unspace";
pub const STATE_DIR: &str = "/var/lib/unspace";
pub const SYSTEMD_UNIT_PATH: &str = "/etc/systemd/system/unspace-dock.service";
pub const UPLOAD_DIR: &str = "/var/lib/unspace/uploads";
pub const QUEUE_DIR: &str = "/var/lib/unspace/queue";
pub const DEFAULT_HEARTBEAT_FILE: &str = "/var/lib/unspace/heartbeat";
