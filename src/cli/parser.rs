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

                if let (Some(dur), Some(per)) = (duration_str, period_str) {
                    match (dur.parse::<u32>(), per.parse::<u32>()) {
                        (Ok(duration_ms), Ok(period_ms)) => {
                            // Validate ranges
                            if duration_ms < 1 || duration_ms > 2000 {
                                CliCommand::Unknown("led_pulse: duration must be 1-2000ms".to_string())
                            } else if period_ms < 3000 || period_ms > 3600000 {
                                CliCommand::Unknown("led_pulse: period must be 3000-3600000ms (3s-1h)".to_string())
                            } else if duration_ms >= period_ms {
                                CliCommand::Unknown("led_pulse: duration must be less than period".to_string())
                            } else {
                                CliCommand::LedPulse(duration_ms, period_ms)
                            }
                        }
                        _ => CliCommand::Unknown("led_pulse: invalid duration or period".to_string())
                    }
                } else {
                    CliCommand::Unknown("led_pulse: requires <duration_ms> <period_ms>".to_string())
                }
            }
            "led_status" => CliCommand::LedStatus,
            "led_blink" => {
                if let Some(freq_str) = parts.next() {
                    if let Ok(frequency_hz) = freq_str.parse::<u32>() {
                        if (1..=10).contains(&frequency_hz) {
                            CliCommand::LedBlink(frequency_hz)
                        } else {
                            CliCommand::Unknown("led_blink: frequency must be 1-10 Hz".to_string())
                        }
                    } else {
                        CliCommand::Unknown("led_blink: invalid frequency".to_string())
                    }
                } else {
                    CliCommand::Unknown("led_blink: frequency (Hz) required".to_string())
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
