use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::path::Path;
use tracing::{info, warn};

/// TOML configuration file structure
#[derive(Debug, Deserialize, Default)]
pub struct TomlConfig {
    pub server: Option<ServerConfig>,
    pub database: Option<DatabaseConfig>,
    pub credentials: Option<CredentialsConfig>,
    pub acme: Option<AcmeTomlConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub port: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CredentialsConfig {
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AcmeTomlConfig {
    pub enabled: Option<bool>,
    pub domains: Option<Vec<String>>,
    pub contact: Option<String>,
    pub cache_dir: Option<String>,
    pub staging: Option<bool>,
    pub http_port: Option<u16>,
    pub https_port: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct AcmeConfig {
    pub domains: Vec<String>,
    pub contact: Option<String>,
    pub cache_dir: String,
    pub staging: bool,
    pub http_port: u16,
    pub https_port: u16,
}

/// Resolved application configuration (after merging TOML + CLI)
#[derive(Debug)]
pub struct AppConfig {
    pub port: u16,
    pub db_path: String,
    pub default_username: Option<String>,
    pub default_password: Option<String>,
    pub command: Option<Commands>,
    pub acme: Option<AcmeConfig>,
}

#[derive(Parser, Debug)]
#[command(name = "kernelci-status", about = "KernelCI status monitoring daemon")]
pub struct Cli {
    /// Path to TOML configuration file
    #[arg(long, default_value = "/etc/kernelci-status.toml")]
    pub config: String,

    /// Port to listen on (overrides config file)
    #[arg(long)]
    pub port: Option<u16>,

    /// Path to SQLite database (overrides config file)
    #[arg(long)]
    pub db_path: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Create a new user (reads password from stdin)
    CreateUser {
        /// Username
        username: String,
    },
}

impl AppConfig {
    /// Load configuration by merging TOML file defaults with CLI overrides.
    ///
    /// Priority (highest to lowest):
    /// 1. CLI arguments
    /// 2. TOML config file
    /// 3. Built-in defaults
    pub fn load(cli: Cli) -> Self {
        let toml_config = load_toml(&cli.config);

        let port_was_set = cli.port.is_some()
            || toml_config.server.as_ref().and_then(|s| s.port).is_some();
        let port = cli
            .port
            .or(toml_config.server.as_ref().and_then(|s| s.port))
            .unwrap_or(2001);

        let db_path = cli
            .db_path
            .or(toml_config.database.as_ref().and_then(|d| d.path.clone()))
            .unwrap_or_else(|| "status.db".to_string());

        let (default_username, default_password) =
            match toml_config.credentials {
                Some(creds) => (creds.username, creds.password),
                None => (None, None),
            };

        let acme = toml_config.acme.and_then(|a| {
            if !a.enabled.unwrap_or(false) {
                return None;
            }
            let domains = a.domains.unwrap_or_default();
            if domains.is_empty() {
                warn!("acme.enabled = true but no domains configured; disabling ACME");
                return None;
            }
            Some(AcmeConfig {
                domains,
                contact: a.contact,
                cache_dir: a
                    .cache_dir
                    .unwrap_or_else(|| "/var/lib/kernelci-status/acme".to_string()),
                staging: a.staging.unwrap_or(true),
                http_port: a.http_port.unwrap_or(80),
                https_port: a.https_port.unwrap_or(443),
            })
        });

        if acme.is_some() && port_was_set {
            warn!(
                "[server].port = {port} is ignored because [acme].enabled = true; \
                 the daemon listens on acme.http_port and acme.https_port instead"
            );
        }

        AppConfig {
            port,
            db_path,
            default_username,
            default_password,
            command: cli.command,
            acme,
        }
    }
}

fn load_toml(path: &str) -> TomlConfig {
    let path = Path::new(path);
    if !path.exists() {
        info!("Config file {} not found, using defaults", path.display());
        return TomlConfig::default();
    }

    match std::fs::read_to_string(path) {
        Ok(content) => match toml::from_str::<TomlConfig>(&content) {
            Ok(config) => {
                info!("Loaded configuration from {}", path.display());
                config
            }
            Err(e) => {
                warn!("Failed to parse {}: {e}, using defaults", path.display());
                TomlConfig::default()
            }
        },
        Err(e) => {
            warn!("Failed to read {}: {e}, using defaults", path.display());
            TomlConfig::default()
        }
    }
}
