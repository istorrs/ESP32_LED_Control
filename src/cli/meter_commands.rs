use super::meter_parser::MeterCommand;
use super::CliError;
use crate::meter::{MeterHandler, MeterType};
use std::sync::Arc;
use std::time::Instant;

pub struct MeterCommandHandler {
    start_time: Instant,
    meter: Option<Arc<MeterHandler>>,
}

impl Default for MeterCommandHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl MeterCommandHandler {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            meter: None,
        }
    }

    pub fn with_meter(mut self, meter: Arc<MeterHandler>) -> Self {
        self.meter = Some(meter);
        self
    }

    pub fn execute_command(&mut self, command: MeterCommand) -> Result<String, CliError> {
        let mut response = String::new();

        match command {
            MeterCommand::Empty => {
                // Empty command - just return empty response (no error)
            }
            MeterCommand::Help => {
                // Help is handled in terminal.rs
                response.push_str("Help displayed");
            }
            MeterCommand::Version => {
                log::info!("CLI: Version requested");
                response.push_str("ESP32 Water Meter Simulator v1.0.0\r\n");
                response.push_str("Built with ESP-IDF");
            }
            MeterCommand::Status => {
                log::info!("CLI: Meter status requested");
                if let Some(ref meter) = self.meter {
                    let config = meter.get_config();
                    let (pulses, bits_tx, messages, transmitting) = meter.get_stats();

                    response.push_str("Meter Status:\r\n");
                    response.push_str(&format!(
                        "  State: {}\r\n",
                        if config.enabled {
                            "Enabled"
                        } else {
                            "Disabled"
                        }
                    ));
                    response.push_str(&format!("  Type: {:?}\r\n", config.meter_type));
                    response.push_str(&format!(
                        "  UART format: {}\r\n",
                        config.uart_format.as_str()
                    ));
                    response.push_str("  Pins: GPIO4 (clock in), GPIO5 (data out)\r\n");
                    response.push_str(&format!(
                        "  Message: '{}' ({} chars)\r\n",
                        config.response_message.as_str(),
                        config.response_message.len()
                    ));
                    response.push_str("  Statistics:\r\n");
                    response.push_str(&format!("    Clock pulses: {}\r\n", pulses));
                    response.push_str(&format!("    Bits transmitted: {}\r\n", bits_tx));
                    response.push_str(&format!("    Messages sent: {}\r\n", messages));
                    response.push_str(&format!(
                        "    Currently transmitting: {}",
                        if transmitting { "Yes" } else { "No" }
                    ));
                } else {
                    response.push_str("Meter not configured");
                }
            }
            MeterCommand::Uptime => {
                log::info!("CLI: Uptime requested");
                let uptime = self.start_time.elapsed();
                let uptime_secs = uptime.as_secs();
                let hours = uptime_secs / 3600;
                let minutes = (uptime_secs % 3600) / 60;
                let seconds = uptime_secs % 60;

                response.push_str("Uptime: ");
                if hours > 0 {
                    response.push_str(&format!("{}h ", hours));
                }
                if minutes > 0 || hours > 0 {
                    response.push_str(&format!("{}m ", minutes));
                }
                response.push_str(&format!("{}s", seconds));
            }
            MeterCommand::Clear => {
                // Clear is handled in terminal.rs
                response.push_str("Screen cleared");
            }
            MeterCommand::Reset => {
                log::info!("CLI: Reset requested");
                response.push_str("Resetting system...");
                // Perform system reset using ESP-IDF
                unsafe {
                    esp_idf_svc::sys::esp_restart();
                }
            }
            MeterCommand::Enable => {
                log::info!("CLI: Meter enable requested");
                if let Some(ref meter) = self.meter {
                    meter.enable();
                    response.push_str("Meter enabled - will respond to clock signals");
                } else {
                    response.push_str("Meter not configured");
                }
            }
            MeterCommand::Disable => {
                log::info!("CLI: Meter disable requested");
                if let Some(ref meter) = self.meter {
                    meter.disable();
                    response.push_str("Meter disabled - will not respond to clock signals");
                } else {
                    response.push_str("Meter not configured");
                }
            }
            MeterCommand::SetType(meter_type) => {
                log::info!("CLI: Meter type set to {:?}", meter_type);
                if let Some(ref meter) = self.meter {
                    meter.set_type(meter_type);
                    let type_str = match meter_type {
                        MeterType::Sensus => "Sensus (7E1: 7 data + even parity + 1 stop)",
                        MeterType::Neptune => "Neptune (7E2: 7 data + even parity + 2 stop)",
                    };
                    response.push_str(&format!("Meter type set to: {}", type_str));
                } else {
                    response.push_str("Meter not configured");
                }
            }
            MeterCommand::SetMessage(text) => {
                log::info!("CLI: Meter message set to: {}", text);
                if let Some(ref meter) = self.meter {
                    // Convert std::string::String to heapless::String
                    let mut heapless_msg = heapless::String::<256>::new();
                    if heapless_msg.push_str(&text).is_ok() {
                        meter.set_message(heapless_msg);
                        response.push_str(&format!(
                            "Meter message set to: '{}' ({} characters)",
                            text,
                            text.len()
                        ));
                    } else {
                        response.push_str("Error: Message too long (max 256 characters)");
                    }
                } else {
                    response.push_str("Meter not configured");
                }
            }
            MeterCommand::SetFormat(format_str) => {
                log::info!("CLI: Meter UART format set to: {}", format_str);
                if let Some(ref meter) = self.meter {
                    if let Ok(format) = format_str.parse::<crate::uart_format::UartFormat>() {
                        meter.set_uart_format(format);
                        response.push_str(&format!("Meter UART format set to: {}", format_str));
                    } else {
                        response.push_str("Invalid UART format");
                    }
                } else {
                    response.push_str("Meter not configured");
                }
            }
            MeterCommand::Unknown(msg) => {
                log::info!("CLI: Unknown command");
                response.push_str(&msg);
            }
        }

        Ok(response)
    }
}
