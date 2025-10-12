pub mod commands;
pub mod parser;
pub mod terminal;

// Meter CLI modules
pub mod meter_commands;
pub mod meter_parser;

pub use commands::CommandHandler;
pub use parser::CommandParser;
pub use terminal::Terminal;

// Meter CLI exports
pub use meter_commands::MeterCommandHandler;
pub use meter_parser::{MeterCommand, MeterCommandParser};

// CLI-related types and constants
pub const CLI_BUFFER_SIZE: usize = 128;
pub const MAX_HISTORY_SIZE: usize = 10;

#[derive(Debug, Clone)]
pub enum CliCommand {
    Help,
    Version,
    Status,
    Uptime,
    Clear,
    Reset,
    Echo(String),
    MtuStart(Option<u16>), // Optional duration in seconds
    MtuStop,
    MtuStatus,
    MtuBaud(u32),                                // Set MTU baud rate
    MtuFormat(String), // Set MTU UART format (7E1, 7E2, 8N1, 8E1, 7O1, 8N2)
    MtuReset,          // Reset MTU statistics
    WifiConnect(Option<String>, Option<String>), // ssid, password (None = use default)
    WifiStatus,
    WifiReconnect,       // Reconnect using stored credentials
    MqttConnect(String), // broker_url
    MqttStatus,
    MqttPublish(String, String), // topic, message
    Empty,
    Unknown(String),
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
