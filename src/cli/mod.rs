pub mod commands;
pub mod parser;
pub mod terminal;

pub use commands::CommandHandler;
pub use parser::CommandParser;
pub use terminal::Terminal;

// CLI-related types and constants
pub const CLI_BUFFER_SIZE: usize = 128;
pub const MAX_HISTORY_SIZE: usize = 10;

#[derive(Debug, Clone)]
pub enum CliCommand {
    // System commands
    Help,
    Version,
    Status,
    Uptime,
    Clear,
    Reset,
    Echo(String),
    // LED commands
    LedOn,
    LedOff,
    LedPulse(u32, u32, u8),  // duration_ms, period_ms, brightness_percent
    LedStatus,
    LedBlink(u32),       // frequency_hz
    // WiFi commands
    WifiConnect(Option<String>, Option<String>), // ssid, password (None = use default)
    WifiStatus,
    WifiReconnect,       // Reconnect using stored credentials
    WifiScan,            // Scan for available WiFi networks
    // MQTT commands
    MqttStatus,
    MqttPublish(String, String), // topic, message
    MqttEnable,
    MqttDisable,
    // Other
    Empty,
    InvalidSyntax(String), // Invalid parameters/syntax for a known command
    Unknown(String),        // Completely unknown command
}

#[derive(Debug)]
pub enum CliError {
    InvalidCommand,
    InvalidArgument,
    UartError,
    BufferFull,
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CliError::InvalidCommand => write!(f, "Invalid command"),
            CliError::InvalidArgument => write!(f, "Invalid argument"),
            CliError::UartError => write!(f, "UART error"),
            CliError::BufferFull => write!(f, "Buffer full"),
        }
    }
}

impl std::error::Error for CliError {}
