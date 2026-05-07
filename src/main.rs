use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use signal_hook::consts::signal::SIGHUP;
use signal_hook::flag as signal_flag;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const CONFIG_VERSION: u32 = 1;
const DEFAULT_CONFIG_PATH: &str = "/etc/unspace/config.json";
const PID_FILE: &str = "/var/run/unspace.pid";
const SYSTEMD_UNIT_PATH: &str = "/etc/systemd/system/unspace.service";
const UPLOAD_DIR: &str = "/var/lib/unspace/uploads";
const QUEUE_DIR: &str = "/var/lib/unspace/queue";

#[derive(Parser, Debug)]
#[command(name = "unspace", version = env!("CARGO_PKG_VERSION"), about = "Unspace drone dock ingest agent")]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand, Debug)]
enum CommandKind {
    /// Install the systemd service and write the initial config.
    Install(InstallArgs),
    /// Stop and remove the systemd service and config.
    Uninstall,
    /// Run the long-lived watcher service.
    Watch,
    /// Show systemd status for the service.
    Status,
    /// Follow journald logs for the service.
    Logs,
    /// Self-update this binary.
    Update(UpdateArgs),
    /// Validate local deployment health.
    Healthcheck,
    /// Show or edit config.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Args, Debug)]
struct InstallArgs {
    #[arg(long)]
    api_key: String,
    #[arg(long)]
    yard_id: String,
    #[arg(long)]
    dock_id: String,
    #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
    config_path: PathBuf,
}

#[derive(Args, Debug)]
struct UpdateArgs {
    #[arg(long)]
    version: Option<String>,
}

#[derive(Subcommand, Debug)]
enum ConfigCommand {
    /// Print current config with the API key redacted.
    Show,
    /// Set one supported config key and signal a running service to reload.
    Set { key: String, value: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct AgentConfig {
    config_version: u32,
    api_base_url: String,
    #[serde(default)]
    api_key: String,
    yard_id: String,
    dock_id: String,
    watch_dir: PathBuf,
    watch_extensions: Vec<String>,
    stable_check_interval_secs: f64,
    stable_required_checks: u32,
    mission_name_prefix: String,
    model_type: String,
    annotated_video: bool,
    multi_track: bool,
    poll_interval_secs: u64,
    max_queue_size_mb: u64,
    heartbeat_file: PathBuf,
    heartbeat_interval_secs: u64,
    heartbeat_max_age_secs: u64,
    log_level: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            config_version: CONFIG_VERSION,
            api_base_url: "https://api.unspace.com".to_string(),
            api_key: String::new(),
            yard_id: String::new(),
            dock_id: String::new(),
            watch_dir: PathBuf::from(UPLOAD_DIR),
            watch_extensions: vec![".mp4", ".mov", ".avi", ".mkv"]
                .into_iter()
                .map(String::from)
                .collect(),
            stable_check_interval_secs: 1.0,
            stable_required_checks: 3,
            mission_name_prefix: "Hextronics Dock".to_string(),
            model_type: "coupler_genie".to_string(),
            annotated_video: true,
            multi_track: false,
            poll_interval_secs: 60,
            max_queue_size_mb: 500,
            heartbeat_file: PathBuf::from("/tmp/unspace.heartbeat"),
            heartbeat_interval_secs: 15,
            heartbeat_max_age_secs: 120,
            log_level: "info".to_string(),
        }
    }
}

impl AgentConfig {
    fn initial(api_key: String, yard_id: String, dock_id: String) -> Self {
        Self {
            api_key,
            yard_id,
            dock_id,
            ..Self::default()
        }
    }

    fn validate(&mut self) -> Result<()> {
        if self.config_version != CONFIG_VERSION {
            bail!(
                "Unsupported config_version {}. Only version {} is supported.",
                self.config_version,
                CONFIG_VERSION
            );
        }
        self.api_base_url = self.api_base_url.trim_end_matches('/').to_string();
        if self.api_base_url.is_empty() {
            bail!("api_base_url is required");
        }
        if self.yard_id.trim().is_empty() {
            bail!("yard_id is required");
        }
        if self.dock_id.trim().is_empty() {
            bail!("dock_id is required");
        }
        let env_api_key = env::var("UNSPACE_API_KEY").unwrap_or_default();
        if !env_api_key.trim().is_empty() {
            self.api_key = env_api_key.trim().to_string();
        }
        if self.api_key.trim().is_empty() {
            bail!("API key is required in config api_key or UNSPACE_API_KEY");
        }
        if !self.api_key.starts_with("ysk_") {
            bail!("API key must start with ysk_");
        }
        if self.stable_check_interval_secs <= 0.0 {
            bail!("stable_check_interval_secs must be greater than 0");
        }
        if self.stable_required_checks == 0 {
            bail!("stable_required_checks must be greater than 0");
        }
        if self.poll_interval_secs == 0 {
            bail!("poll_interval_secs must be greater than 0");
        }
        if self.heartbeat_interval_secs == 0 {
            bail!("heartbeat_interval_secs must be greater than 0");
        }
        if self.heartbeat_max_age_secs == 0 {
            bail!("heartbeat_max_age_secs must be greater than 0");
        }
        self.watch_extensions = normalize_extensions(&self.watch_extensions)?;
        Ok(())
    }

    fn redacted_value(&self) -> Result<Value> {
        let mut value = serde_json::to_value(self)?;
        value["api_key"] = if self.api_key.is_empty() {
            json!("")
        } else {
            json!("<redacted>")
        };
        Ok(value)
    }
}

fn main() {
    init_logging("info");
    if let Err(error) = run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        CommandKind::Install(args) => install(args),
        CommandKind::Uninstall => uninstall(),
        CommandKind::Watch => watch(),
        CommandKind::Status => exec_system("systemctl", &["status", "unspace"]),
        CommandKind::Logs => exec_system("journalctl", &["-u", "unspace", "-f"]),
        CommandKind::Update(args) => update(args),
        CommandKind::Healthcheck => healthcheck(),
        CommandKind::Config { command } => match command {
            ConfigCommand::Show => config_show(),
            ConfigCommand::Set { key, value } => config_set(&key, &value),
        },
    }
}

fn init_logging(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

fn config_path() -> PathBuf {
    env::var("UNSPACE_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_CONFIG_PATH))
}

fn load_config() -> Result<AgentConfig> {
    load_config_from(&config_path())
}

fn load_config_from(path: &Path) -> Result<AgentConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read config at {}", path.display()))?;
    let raw: Value = serde_json::from_str(&contents).context("config is not valid JSON")?;
    match raw.get("config_version").and_then(Value::as_u64) {
        Some(1) => {}
        Some(other) => bail!("Unsupported config_version {other}. Only version 1 is supported."),
        None => bail!("config_version is required and must be 1"),
    }
    let mut config: AgentConfig =
        serde_json::from_value(raw).context("config schema is invalid")?;
    config.validate()?;
    Ok(config)
}

fn write_config(path: &Path, config: &AgentConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    let serialized = serde_json::to_string_pretty(config)?;
    fs::write(path, format!("{serialized}\n"))
        .with_context(|| format!("failed to write config at {}", path.display()))
}

fn install(args: InstallArgs) -> Result<()> {
    let mut config = AgentConfig::initial(args.api_key, args.yard_id, args.dock_id);
    config.validate()?;
    write_config(&args.config_path, &config)?;

    fs::create_dir_all(UPLOAD_DIR).context("failed to create upload directory")?;
    fs::create_dir_all(QUEUE_DIR).context("failed to create queue directory")?;
    let _ = Command::new("groupadd")
        .args(["--system", "unspace"])
        .status();
    let _ = Command::new("useradd")
        .args([
            "--system",
            "--gid",
            "unspace",
            "--home-dir",
            "/var/lib/unspace",
            "--shell",
            "/usr/sbin/nologin",
            "unspace",
        ])
        .status();
    let _ = Command::new("chown")
        .args(["-R", "unspace:unspace", "/var/lib/unspace"])
        .status();

    fs::write(SYSTEMD_UNIT_PATH, systemd_unit(&args.config_path))
        .context("failed to write systemd unit")?;
    exec_system("systemctl", &["daemon-reload"])?;
    exec_system("systemctl", &["enable", "--now", "unspace"])?;
    println!("Unspace service installed and started.");
    Ok(())
}

fn uninstall() -> Result<()> {
    let _ = Command::new("systemctl").args(["stop", "unspace"]).status();
    let _ = Command::new("systemctl")
        .args(["disable", "unspace"])
        .status();
    remove_if_exists(Path::new(SYSTEMD_UNIT_PATH))?;
    remove_if_exists(&config_path())?;
    remove_if_exists(Path::new(PID_FILE))?;
    exec_system("systemctl", &["daemon-reload"])?;
    println!("Unspace service uninstalled.");
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

fn systemd_unit(config_path: &Path) -> String {
    format!(
        r#"[Unit]
Description=Unspace Drone Dock CLI
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=unspace
Group=unspace
Environment=UNSPACE_CONFIG_PATH={}
ExecStartPre=/usr/local/bin/unspace healthcheck
ExecStart=/usr/local/bin/unspace watch
ExecStartPost=/usr/local/bin/unspace healthcheck
Restart=always
RestartSec=5
StartLimitIntervalSec=300
StartLimitBurst=5
WatchdogSec=60
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full
ProtectHome=true
ReadWritePaths=/tmp /var/lib/unspace/uploads /var/lib/unspace/queue
StandardOutput=journal
StandardError=journal
SyslogIdentifier=unspace

[Install]
WantedBy=multi-user.target
"#,
        config_path.display()
    )
}

fn watch() -> Result<()> {
    let mut config = load_config()?;
    init_logging(&config.log_level);
    write_pid_file()?;
    let reload = Arc::new(AtomicBool::new(false));
    signal_flag::register(SIGHUP, Arc::clone(&reload))
        .context("failed to register SIGHUP handler")?;
    info!(watch_dir = %config.watch_dir.display(), "watch service started");
    let mut heartbeat_elapsed = config.heartbeat_interval_secs;

    loop {
        if reload.swap(false, Ordering::Relaxed) {
            match load_config() {
                Ok(new_config) => {
                    info!(old_watch_dir = %config.watch_dir.display(), new_watch_dir = %new_config.watch_dir.display(), "config reloaded");
                    config = new_config;
                }
                Err(error) => {
                    warn!(error = %error, "failed to reload config; keeping previous config")
                }
            }
        }

        if heartbeat_elapsed >= config.heartbeat_interval_secs {
            write_heartbeat(&config.heartbeat_file)?;
            heartbeat_elapsed = 0;
        }

        thread::sleep(Duration::from_secs(1));
        heartbeat_elapsed += 1;
    }
}

fn write_pid_file() -> Result<()> {
    fs::write(PID_FILE, std::process::id().to_string()).context("failed to write PID file")
}

fn write_heartbeat(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create heartbeat directory {}", parent.display())
        })?;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    fs::write(path, now.to_string())
        .with_context(|| format!("failed to write heartbeat {}", path.display()))
}

fn config_show() -> Result<()> {
    let config = load_config()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&config.redacted_value()?)?
    );
    Ok(())
}

fn config_set(key: &str, value: &str) -> Result<()> {
    let path = config_path();
    let mut config = load_config_from(&path)?;
    apply_config_value(&mut config, key, value)?;
    config.validate()?;
    write_config(&path, &config)?;
    if signal_running_service()? {
        println!("Config updated. Service reloaded.");
    } else {
        println!("Config updated. Service not running.");
    }
    Ok(())
}

fn apply_config_value(config: &mut AgentConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "api_base_url" => config.api_base_url = value.to_string(),
        "api_key" => config.api_key = value.to_string(),
        "yard_id" => config.yard_id = value.to_string(),
        "dock_id" => config.dock_id = value.to_string(),
        "watch_dir" => config.watch_dir = PathBuf::from(value),
        "watch_extensions" => config.watch_extensions = parse_extensions(value),
        "stable_check_interval_secs" => config.stable_check_interval_secs = value.parse()?,
        "stable_required_checks" => config.stable_required_checks = value.parse()?,
        "mission_name_prefix" => config.mission_name_prefix = value.to_string(),
        "model_type" => config.model_type = value.to_string(),
        "annotated_video" => config.annotated_video = parse_bool(value)?,
        "multi_track" => config.multi_track = parse_bool(value)?,
        "poll_interval_secs" => config.poll_interval_secs = value.parse()?,
        "max_queue_size_mb" => config.max_queue_size_mb = value.parse()?,
        "heartbeat_file" => config.heartbeat_file = PathBuf::from(value),
        "heartbeat_interval_secs" => config.heartbeat_interval_secs = value.parse()?,
        "heartbeat_max_age_secs" => config.heartbeat_max_age_secs = value.parse()?,
        "log_level" => config.log_level = value.to_string(),
        "config_version" => bail!("config_version cannot be changed with config set"),
        _ => bail!("unsupported config key '{key}'"),
    }
    Ok(())
}

fn normalize_extensions(values: &[String]) -> Result<Vec<String>> {
    let extensions: Vec<String> = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| format!(".{}", value.trim_start_matches('.').to_ascii_lowercase()))
        .collect();
    if extensions.is_empty() {
        bail!("watch_extensions must include at least one extension");
    }
    Ok(extensions)
}

fn parse_extensions(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|part| part.trim().to_string())
        .collect()
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Ok(true),
        "0" | "false" | "no" | "n" | "off" => Ok(false),
        _ => Err(anyhow!("expected a boolean value")),
    }
}

fn signal_running_service() -> Result<bool> {
    let pid = match fs::read_to_string(PID_FILE) {
        Ok(pid) => pid.trim().to_string(),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("failed to read PID file"),
    };
    if pid.is_empty() {
        return Ok(false);
    }
    let status = Command::new("kill")
        .args(["-HUP", &pid])
        .status()
        .context("failed to execute kill -HUP")?;
    Ok(status.success())
}

fn healthcheck() -> Result<()> {
    let config = load_config()?;
    if !config.watch_dir.exists() {
        bail!(
            "watch directory does not exist: {}",
            config.watch_dir.display()
        );
    }
    if !config.watch_dir.is_dir() {
        bail!(
            "watch path is not a directory: {}",
            config.watch_dir.display()
        );
    }
    let _ = fs::read_dir(&config.watch_dir).with_context(|| {
        format!(
            "watch directory is not readable: {}",
            config.watch_dir.display()
        )
    })?;
    check_heartbeat(&config)?;
    Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?
        .get(format!("{}/health", config.api_base_url))
        .send()
        .context("API /health is unreachable")?
        .error_for_status()
        .context("API /health returned an error status")?;
    println!("Healthcheck passed.");
    Ok(())
}

fn check_heartbeat(config: &AgentConfig) -> Result<()> {
    if !config.heartbeat_file.exists() {
        bail!(
            "heartbeat file does not exist: {}",
            config.heartbeat_file.display()
        );
    }
    let contents = fs::read_to_string(&config.heartbeat_file).with_context(|| {
        format!(
            "failed to read heartbeat file {}",
            config.heartbeat_file.display()
        )
    })?;
    let timestamp: u64 = contents
        .trim()
        .parse()
        .context("heartbeat file does not contain a Unix timestamp")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let age = now.saturating_sub(timestamp);
    if age > config.heartbeat_max_age_secs {
        bail!(
            "heartbeat is stale ({age}s old, max {}s)",
            config.heartbeat_max_age_secs
        );
    }
    Ok(())
}

fn update(args: UpdateArgs) -> Result<()> {
    let requested = args.version.as_deref().unwrap_or("latest");
    println!("checking for {requested} update...");
    bail!("self-update download/apply flow is not implemented yet")
}

fn exec_system(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to execute {program}"))?;
    if !status.success() {
        bail!("{program} exited with status {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_extensions_lowercases_and_adds_dots() {
        let values = vec!["MP4".to_string(), ".Mov".to_string(), " avi ".to_string()];
        assert_eq!(
            normalize_extensions(&values).unwrap(),
            vec![".mp4", ".mov", ".avi"]
        );
    }

    #[test]
    fn config_validation_rejects_missing_api_key() {
        let mut config = AgentConfig {
            yard_id: "yard".to_string(),
            dock_id: "dock".to_string(),
            api_key: String::new(),
            ..AgentConfig::default()
        };
        unsafe { env::remove_var("UNSPACE_API_KEY") };
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("API key is required")
        );
    }

    #[test]
    fn config_value_set_parses_extensions() {
        let mut config = AgentConfig::initial(
            "ysk_test".to_string(),
            "yard".to_string(),
            "dock".to_string(),
        );
        apply_config_value(&mut config, "watch_extensions", ".MP4,mov").unwrap();
        config.validate().unwrap();
        assert_eq!(config.watch_extensions, vec![".mp4", ".mov"]);
    }

    #[test]
    fn load_config_rejects_unsupported_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"config_version":2}"#).unwrap();
        assert!(
            load_config_from(&path)
                .unwrap_err()
                .to_string()
                .contains("Unsupported config_version 2")
        );
    }

    #[test]
    fn redacted_config_hides_api_key() {
        let config = AgentConfig::initial(
            "ysk_secret".to_string(),
            "yard".to_string(),
            "dock".to_string(),
        );
        assert_eq!(
            config.redacted_value().unwrap()["api_key"],
            json!("<redacted>")
        );
    }
}
