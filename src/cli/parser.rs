use super::CliCommand;

pub struct CommandParser;

impl Default for CommandParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandParser {
    pub fn new() -> Self {
        Self
    }

    /// Parse duration string with optional unit suffix (us, ms, s)
    /// Returns duration in microseconds
    /// Examples: "500" -> 500μs, "500us" -> 500μs, "10ms" -> 10000μs, "1s" -> 1000000μs
    fn parse_duration(s: &str) -> Result<u32, String> {
        if s.ends_with("us") || s.ends_with("μs") {
            // Microseconds
            let num_str = s.trim_end_matches("us").trim_end_matches("μs");
            num_str.parse::<u32>()
                .map_err(|_| "invalid number".to_string())
        } else if s.ends_with("ms") {
            // Milliseconds -> microseconds
            let num_str = s.trim_end_matches("ms");
            num_str.parse::<u32>()
                .map(|ms| ms * 1000)
                .map_err(|_| "invalid number".to_string())
        } else if s.ends_with("s") {
            // Seconds -> microseconds
            let num_str = s.trim_end_matches("s");
            num_str.parse::<u32>()
                .map(|s| s * 1_000_000)
                .map_err(|_| "invalid number".to_string())
        } else {
            // No suffix - assume microseconds for backward compatibility with millisecond values
            // If value looks like milliseconds (>=1000), convert to microseconds
            match s.parse::<u32>() {
                Ok(val) if val >= 1000 => Ok(val * 1000), // Assume ms, convert to μs
                Ok(val) => Ok(val), // Small value, assume μs
                Err(_) => Err("invalid number".to_string())
            }
        }
    }

    pub fn get_available_commands() -> &'static [&'static str] {
        &[
            "help",
            "version",
            "status",
            "uptime",
            "clear",
            "reset",
            "echo",
            "led_on",
            "led_off",
            "led_pulse",
            "led_status",
            "led_blink",
            "wifi_connect",
            "wifi_reconnect",
            "wifi_status",
            "wifi_scan",
            "mqtt_status",
            "mqtt_enable",
            "mqtt_disable",
            "mqtt_publish",
        ]
    }

    pub fn autocomplete(partial: &str) -> Vec<&'static str> {
        let commands = Self::get_available_commands();
        commands
            .iter()
            .filter(|&&cmd| cmd.starts_with(partial))
            .copied()
            .collect()
    }

    pub fn parse_command(input: &str) -> CliCommand {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return CliCommand::Empty;
        }

        let mut parts = trimmed.split_whitespace();
        let cmd = parts.next().unwrap_or("");

        match cmd {
            "help" => CliCommand::Help,
            "version" => CliCommand::Version,
            "status" => CliCommand::Status,
            "uptime" => CliCommand::Uptime,
            "clear" => CliCommand::Clear,
            "reset" => CliCommand::Reset,
            "led_on" => CliCommand::LedOn,
            "led_off" => CliCommand::LedOff,
            "led_pulse" => {
                let duration_str = parts.next();
                let period_str = parts.next();
                let brightness_str = parts.next();

                // Brightness is optional, defaults to 75%
                let brightness_percent = if let Some(b) = brightness_str {
                    match b.parse::<u8>() {
                        Ok(val) if val <= 100 => val,
                        _ => return CliCommand::InvalidSyntax("brightness must be 0-100%".to_string()),
                    }
                } else {
                    75 // Default brightness
                };

                if let (Some(dur), Some(per)) = (duration_str, period_str) {
                    match (Self::parse_duration(dur), Self::parse_duration(per)) {
                        (Ok(duration_us), Ok(period_us)) => {
                            // Validation is handled by PulseConfig::new, so just pass through
                            CliCommand::LedPulse(duration_us, period_us, brightness_percent)
                        }
                        (Err(e), _) => CliCommand::InvalidSyntax(format!("invalid duration: {}", e)),
                        (_, Err(e)) => CliCommand::InvalidSyntax(format!("invalid period: {}", e)),
                    }
                } else {
                    CliCommand::InvalidSyntax("requires <duration> <period> [brightness_%]\r\nExamples: 500us 5ms, 10ms 1s, 100 5000".to_string())
                }
            }
            "led_status" => CliCommand::LedStatus,
            "led_blink" => {
                if let Some(freq_str) = parts.next() {
                    if let Ok(frequency_hz) = freq_str.parse::<u32>() {
                        if (1..=10).contains(&frequency_hz) {
                            CliCommand::LedBlink(frequency_hz)
                        } else {
                            CliCommand::InvalidSyntax("frequency must be 1-10 Hz".to_string())
                        }
                    } else {
                        CliCommand::InvalidSyntax("invalid frequency value".to_string())
                    }
                } else {
                    CliCommand::InvalidSyntax("frequency (Hz) required".to_string())
                }
            }
            "echo" => {
                let args: Vec<&str> = parts.collect();
                let echo_string = args.join(" ");
                CliCommand::Echo(echo_string)
            }
            "wifi_connect" => {
                let ssid = parts.next().map(|s| s.to_string());
                let password = parts.next().map(|s| s.to_string());
                CliCommand::WifiConnect(ssid, password)
            }
            "wifi_reconnect" => CliCommand::WifiReconnect,
            "wifi_status" => CliCommand::WifiStatus,
            "wifi_scan" => CliCommand::WifiScan,
            "mqtt_status" => CliCommand::MqttStatus,
            "mqtt_enable" => CliCommand::MqttEnable,
            "mqtt_disable" => CliCommand::MqttDisable,
            "mqtt_publish" => {
                let topic = parts.next().unwrap_or("").to_string();
                let message_parts: Vec<&str> = parts.collect();
                let message = message_parts.join(" ");
                if topic.is_empty() {
                    CliCommand::Unknown("mqtt_publish: topic required".to_string())
                } else if message.is_empty() {
                    CliCommand::Unknown("mqtt_publish: message required".to_string())
                } else {
                    CliCommand::MqttPublish(topic, message)
                }
            }
            _ => CliCommand::Unknown(cmd.to_string()),
        }
    }
}
