pub type Result<T> = std::result::Result<T, TermzzzError>;

#[derive(Debug, thiserror::Error)]
pub enum TermzzzError {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Unsupported check effect: {0}")]
    UnsupportedEffect(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    FileRead(#[from] std::io::Error),

    #[error("Failed to deserialize config: {0}")]
    DeserializeFormat(#[from] toml::de::Error),

    #[error("Failed to serialize config: {0}")]
    SerializeFormat(#[from] toml::ser::Error),
}
