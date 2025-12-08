use super::{CliCommand, CliError};
use crate::led::{LedManager, LedStatus, PulseConfig};
use crate::mqtt::MqttClient;
use crate::wifi::WifiManager;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct CommandHandler {
    start_time: Instant,
    wifi: Option<Arc<Mutex<WifiManager>>>,
    mqtt: Option<Arc<MqttClient>>,
    led: Option<Arc<LedManager>>,
    event_config_topic: Option<String>,
    event_error_topic: Option<String>,
    mqtt_publishing_enabled: bool,
}

impl Default for CommandHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            wifi: None,
            mqtt: None,
            led: None,
            event_config_topic: None,
            event_error_topic: None,
            mqtt_publishing_enabled: false, // Default to disabled
        }
    }

    /// Format microseconds duration as human-readable string
    fn format_duration(microseconds: u32) -> String {
        if microseconds >= 1_000_000 {
            format!("{:.1}s", microseconds as f32 / 1_000_000.0)
        } else if microseconds >= 1_000 {
            format!("{}ms", microseconds / 1_000)
        } else {
            format!("{}μs", microseconds)
        }
    }

    pub fn with_wifi(mut self, wifi: Arc<Mutex<WifiManager>>) -> Self {
        self.wifi = Some(wifi);
        self
    }

    pub fn with_mqtt(mut self, mqtt: Arc<MqttClient>) -> Self {
        self.mqtt = Some(mqtt);
        self
    }

    pub fn with_led(mut self, led: Arc<LedManager>) -> Self {
        self.led = Some(led);
        self
    }

    pub fn with_event_topics(mut self, config_topic: String, error_topic: String) -> Self {
        self.event_config_topic = Some(config_topic);
        self.event_error_topic = Some(error_topic);
        self
    }

    pub fn execute_command(&mut self, command: CliCommand) -> Result<String, CliError> {
        let mut response = String::new();

        match command {
            CliCommand::Empty => {
                // Empty command - just return empty response (no error)
            }
            CliCommand::Help => {
                log::info!("CLI: Help requested");
                response.push_str("Available commands:\r\n");
                response.push_str("  help        - Show this help\r\n");
                response.push_str("  version     - Show firmware version\r\n");
                response.push_str("  status      - Show system status\r\n");
                response.push_str("  uptime      - Show system uptime\r\n");
                response.push_str("  clear       - Clear terminal\r\n");
                response.push_str("  reset       - Reset system\r\n");
                response.push_str("  echo <text> - Echo text back\r\n");
                response.push_str("  led_on      - Turn LED on (solid)\r\n");
                response.push_str("  led_off     - Turn LED off\r\n");
                response.push_str("  led_pulse <duration> <period> [brightness_%] - Set custom pulse\r\n");
                response.push_str("              Supports us/ms/s units (e.g., 500us 5ms, 10ms 1s)\r\n");
                response.push_str("              Range: 100us-2s duration, 500us-1h period\r\n");
                response.push_str("  led_status  - Show LED status and configuration\r\n");
                response.push_str("  led_blink <hz> - Set blink frequency (1-10 Hz)\r\n");
                response.push_str("  wifi_connect [ssid] [password] - Connect to WiFi (no args = default)\r\n");
                response.push_str("  wifi_reconnect - Quick reconnect to default WiFi\r\n");
                response.push_str("  wifi_status - Show WiFi connection status\r\n");
                response.push_str("  wifi_scan   - Scan for available WiFi networks\r\n");
                response.push_str("  mqtt_status - Show MQTT connection status\r\n");
                response.push_str("  mqtt_enable - Enable MQTT publishing from CLI commands\r\n");
                response.push_str("  mqtt_disable - Disable MQTT publishing from CLI commands\r\n");
                response.push_str("  mqtt_publish <topic> <message> - Publish MQTT message\r\n");
                response.push_str("\r\n");
                response.push_str("Use TAB to autocomplete commands\r\n");
                response.push_str("Use UP/DOWN arrows to navigate command history\r\n");
                response.push_str("Use LEFT/RIGHT arrows to move cursor and edit");
            }
            CliCommand::Version => {
                log::info!("CLI: Version requested");
                response.push_str("ESP32 LED Flasher v1.0.0\r\n");
                response.push_str("Built with ESP-IDF");
            }
            CliCommand::Status => {
                log::info!("CLI: Status requested");
                response.push_str("System Status:\r\n");
                response.push_str("  Firmware: ESP32 LED Flasher v1.0.0\r\n");
                response.push_str("  Platform: ESP32 with ESP-IDF\r\n");
                response.push_str("  LED: GPIO2 (built-in LED)\r\n");
                response.push_str("  UART: USB-C (UART0)");
            }
            CliCommand::Uptime => {
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
            CliCommand::Clear => {
                // Clear is handled in terminal.rs
                response.push_str("Screen cleared");
            }
            CliCommand::Reset => {
                log::info!("CLI: Reset requested");
                response.push_str("Resetting system...");
                // Perform system reset using ESP-IDF
                unsafe {
                    esp_idf_svc::sys::esp_restart();
                }
            }
            CliCommand::Echo(text) => {
                log::info!("CLI: Echo requested: {}", text);
                response.push_str(&text);
            }
            // LED Commands
            CliCommand::LedOn => {
                log::info!("CLI: LED on requested");
                if let Some(ref led) = self.led {
                    led.turn_on();
                    response.push_str("LED turned ON");

                    // Publish config change event to MQTT (if enabled)
                    if self.mqtt_publishing_enabled {
                        if let Some(ref mqtt) = self.mqtt {
                            let config_json = format!(
                                r#"{{"timestamp":{},"source":"cli","event":"config_changed","config":{{"state":"on"}}}}"#,
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs()
                            );
                            if let Some(ref topic) = self.event_config_topic {
                                let _ = mqtt.publish(topic, config_json.as_bytes(), esp_idf_svc::mqtt::client::QoS::AtMostOnce, false);
                            }
                        }
                    }
                } else {
                    response.push_str("LED not initialized");
                }
            }
            CliCommand::LedOff => {
                log::info!("CLI: LED off requested");
                if let Some(ref led) = self.led {
                    led.turn_off();
                    response.push_str("LED turned OFF");

                    // Publish config change event to MQTT (if enabled)
                    if self.mqtt_publishing_enabled {
                        if let Some(ref mqtt) = self.mqtt {
                            let config_json = format!(
                                r#"{{"timestamp":{},"source":"cli","event":"config_changed","config":{{"state":"off"}}}}"#,
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs()
                            );
                            if let Some(ref topic) = self.event_config_topic {
                                let _ = mqtt.publish(topic, config_json.as_bytes(), esp_idf_svc::mqtt::client::QoS::AtMostOnce, false);
                            }
                        }
                    }
                } else {
                    response.push_str("LED not initialized");
                }
            }
            CliCommand::LedPulse(duration_us, period_us, brightness_percent) => {
                log::info!(
                    "CLI: LED pulse requested - duration: {}μs, period: {}μs, brightness: {}%",
                    duration_us,
                    period_us,
                    brightness_percent
                );
                if let Some(ref led) = self.led {
                    match PulseConfig::new(duration_us, period_us, brightness_percent) {
                        Ok(config) => {
                            led.set_pulse(config);
                            response.push_str(&format!(
                                "LED pulse set: {} ON @ {}% / {} period",
                                Self::format_duration(duration_us),
                                brightness_percent,
                                Self::format_duration(period_us)
                            ));

                            // Publish config change event to MQTT (if enabled)
                            if self.mqtt_publishing_enabled {
                                if let Some(ref mqtt) = self.mqtt {
                                    let config_json = format!(
                                        r#"{{"timestamp":{},"source":"cli","event":"config_changed","config":{{"state":"pulsing","duration_us":{},"period_us":{},"brightness_percent":{}}}}}"#,
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs(),
                                        duration_us,
                                        period_us,
                                        brightness_percent
                                    );

                                    if let Some(ref topic) = self.event_config_topic {
                                        let _ = mqtt.publish(topic, config_json.as_bytes(), esp_idf_svc::mqtt::client::QoS::AtMostOnce, false);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            response.push_str(&format!("Invalid pulse configuration: {}", e));
                        }
                    }
                } else {
                    response.push_str("LED not initialized");
                }
            }
            CliCommand::LedStatus => {
                log::info!("CLI: LED status requested");
                if let Some(ref led) = self.led {
                    let status = led.get_status();
                    let stats = led.get_statistics();

                    response.push_str("LED Status:\r\n");
                    match status {
                        LedStatus::Off => {
                            response.push_str("  State: OFF");
                        }
                        LedStatus::SolidOn => {
                            response.push_str("  State: ON (solid)");
                        }
                        LedStatus::CustomPulse(config) => {
                            response.push_str(&format!(
                                "  State: Pulsing\r\n  Duration: {}\r\n  Period: {}\r\n  Brightness: {}%",
                                Self::format_duration(config.duration_us),
                                Self::format_duration(config.period_us),
                                config.brightness_percent
                            ));
                            let duty_cycle =
                                (config.duration_us as f32 / config.period_us as f32) * 100.0;
                            response.push_str(&format!("\r\n  Duty cycle: {:.1}%", duty_cycle));
                        }
                        LedStatus::SlowBlink => {
                            response.push_str("  State: Slow blink (1 Hz)");
                        }
                        LedStatus::FastBlink => {
                            response.push_str("  State: Fast blink (5 Hz)");
                        }
                    }

                    // Add statistics
                    let elapsed_secs = stats.config_changed_time.elapsed().as_secs_f32();
                    response.push_str(&format!("\r\n\r\nStatistics:\r\n  Pulses since config change: {}", stats.pulse_count));
                    response.push_str(&format!("\r\n  Config changed: {:.1}s ago", elapsed_secs));

                    // Calculate actual pulse rate from ISR timestamps
                    // Use time between first and last pulse for accuracy
                    if stats.pulse_count > 1 {
                        if let (Some(first), Some(last)) = (stats.first_pulse_time, stats.last_pulse_time) {
                            let measurement_duration = last.duration_since(first).as_secs_f32();
                            if measurement_duration > 0.0 {
                                // Rate = (pulses - 1) / time_elapsed
                                // We subtract 1 because we're measuring intervals between pulses
                                let actual_rate = (stats.pulse_count - 1) as f32 / measurement_duration;
                                response.push_str(&format!("\r\n  Measured pulse rate: {:.4} Hz", actual_rate));
                                response.push_str(&format!("\r\n  Measurement period: {:.2}s ({} pulses)", measurement_duration, stats.pulse_count));

                                // Compare to expected rate based on period
                                if let LedStatus::CustomPulse(config) = status {
                                    let expected_rate = 1_000_000.0 / config.period_us as f32;
                                    response.push_str(&format!("\r\n  Expected pulse rate: {:.4} Hz", expected_rate));
                                    let error_percent = ((actual_rate - expected_rate) / expected_rate) * 100.0;
                                    response.push_str(&format!("\r\n  Timing accuracy: {:.3}%", error_percent));
                                } else if let LedStatus::SlowBlink = status {
                                    response.push_str(&format!("\r\n  Expected pulse rate: 1.0000 Hz"));
                                    let error_percent = ((actual_rate - 1.0) / 1.0) * 100.0;
                                    response.push_str(&format!("\r\n  Timing accuracy: {:.3}%", error_percent));
                                } else if let LedStatus::FastBlink = status {
                                    response.push_str(&format!("\r\n  Expected pulse rate: 5.0000 Hz"));
                                    let error_percent = ((actual_rate - 5.0) / 5.0) * 100.0;
                                    response.push_str(&format!("\r\n  Timing accuracy: {:.3}%", error_percent));
                                }
                            }
                        }
                    } else if stats.pulse_count == 1 {
                        response.push_str(&format!("\r\n  (Need at least 2 pulses to measure rate)"));
                    }

                    if let Some(last_pulse) = stats.last_pulse_time {
                        response.push_str(&format!("\r\n  Last pulse: {:.1}s ago", last_pulse.elapsed().as_secs_f32()));
                    }

                    // Duty cycle verification from ON/OFF time accumulation
                    let total_measured_time_us = stats.total_on_time_us + stats.total_off_time_us;
                    if total_measured_time_us > 0 {
                        let measured_duty_cycle = (stats.total_on_time_us as f32 / total_measured_time_us as f32) * 100.0;
                        response.push_str(&format!("\r\n\r\nDuty Cycle Verification:"));
                        response.push_str(&format!("\r\n  Total ON time: {:.3}s", stats.total_on_time_us as f32 / 1_000_000.0));
                        response.push_str(&format!("\r\n  Total OFF time: {:.3}s", stats.total_off_time_us as f32 / 1_000_000.0));
                        response.push_str(&format!("\r\n  Measured duty cycle: {:.2}%", measured_duty_cycle));

                        // Compare to expected duty cycle
                        if let LedStatus::CustomPulse(config) = status {
                            let expected_duty_cycle = (config.duration_us as f32 / config.period_us as f32) * 100.0;
                            response.push_str(&format!("\r\n  Expected duty cycle: {:.2}%", expected_duty_cycle));
                            let duty_error = ((measured_duty_cycle - expected_duty_cycle) / expected_duty_cycle) * 100.0;
                            response.push_str(&format!("\r\n  Duty cycle accuracy: {:.3}%", duty_error));
                        } else if let LedStatus::SlowBlink = status {
                            response.push_str(&format!("\r\n  Expected duty cycle: 50.00%"));
                            let duty_error = ((measured_duty_cycle - 50.0) / 50.0) * 100.0;
                            response.push_str(&format!("\r\n  Duty cycle accuracy: {:.3}%", duty_error));
                        } else if let LedStatus::FastBlink = status {
                            response.push_str(&format!("\r\n  Expected duty cycle: 50.00%"));
                            let duty_error = ((measured_duty_cycle - 50.0) / 50.0) * 100.0;
                            response.push_str(&format!("\r\n  Duty cycle accuracy: {:.3}%", duty_error));
                        }
                    }
                } else {
                    response.push_str("LED not initialized");
                }
            }
            CliCommand::LedBlink(frequency_hz) => {
                log::info!("CLI: LED blink requested - frequency: {}Hz", frequency_hz);
                if let Some(ref led) = self.led {
                    led.set_blink(frequency_hz);
                    response.push_str(&format!("LED blink set to {} Hz", frequency_hz));
                } else {
                    response.push_str("LED not initialized");
                }
            }
            // WiFi Commands
            CliCommand::WifiConnect(ssid, password) => {
                log::info!("CLI: WiFi connect requested");
                if let Some(ref wifi) = self.wifi {
                    let ssid_ref = ssid.as_deref();
                    let password_ref = password.as_deref();

                    match wifi.lock() {
                        Ok(mut wifi_guard) => match wifi_guard.reconnect(ssid_ref, password_ref) {
                            Ok(_) => {
                                if ssid.is_none() {
                                    response.push_str("✅ WiFi reconnected to default network");
                                } else {
                                    response.push_str(&format!(
                                        "✅ WiFi connected to: {}",
                                        ssid.as_ref().unwrap()
                                    ));
                                }
                            }
                            Err(e) => {
                                response.push_str(&format!("❌ WiFi connection failed: {:?}", e));
                            }
                        },
                        Err(_) => {
                            response.push_str("❌ WiFi manager lock error");
                        }
                    }
                } else {
                    response.push_str("❌ WiFi not initialized");
                }
            }
            CliCommand::WifiReconnect => {
                log::info!("CLI: WiFi reconnect requested");
                if let Some(ref wifi) = self.wifi {
                    match wifi.lock() {
                        Ok(mut wifi_guard) => match wifi_guard.reconnect(None, None) {
                            Ok(_) => {
                                response.push_str("✅ WiFi reconnected to default network");
                            }
                            Err(e) => {
                                response.push_str(&format!("❌ WiFi reconnect failed: {:?}", e));
                            }
                        },
                        Err(_) => {
                            response.push_str("❌ WiFi manager lock error");
                        }
                    }
                } else {
                    response.push_str("❌ WiFi not initialized");
                }
            }
            CliCommand::WifiStatus => {
                log::info!("CLI: WiFi status requested");
                if let Some(ref wifi) = self.wifi {
                    match wifi.lock() {
                        Ok(wifi_guard) => match wifi_guard.is_connected() {
                            Ok(connected) => {
                                if connected {
                                    if let Ok(ip) = wifi_guard.get_ip() {
                                        response.push_str(&format!(
                                            "WiFi Status: Connected\r\nIP: {}",
                                            ip
                                        ));
                                        if let Ok(ssid) = wifi_guard.get_ssid() {
                                            response
                                                .push_str(&format!("\r\nSSID: {}", ssid.as_str()));
                                        }
                                    } else {
                                        response
                                            .push_str("WiFi Status: Connected (IP unavailable)");
                                    }
                                } else {
                                    response.push_str("WiFi Status: Disconnected");
                                }
                            }
                            Err(_) => {
                                response.push_str("WiFi Status: Error checking connection");
                            }
                        },
                        Err(_) => {
                            response.push_str("WiFi Status: Lock error");
                        }
                    }
                } else {
                    response.push_str("WiFi Status: Not initialized");
                }
            }
            CliCommand::WifiScan => {
                log::info!("CLI: WiFi scan requested");
                if let Some(ref wifi) = self.wifi {
                    match wifi.lock() {
                        Ok(mut wifi_guard) => {
                            response.push_str("Scanning for WiFi networks...\r\n");
                            match wifi_guard.scan() {
                                Ok(aps) => {
                                    if aps.is_empty() {
                                        response.push_str("\r\nNo networks found");
                                    } else {
                                        response.push_str(&format!(
                                            "\r\nFound {} networks:\r\n\r\n",
                                            aps.len()
                                        ));

                                        // Format as table
                                        response.push_str(
                                            "SSID                            | RSSI | Ch\r\n",
                                        );
                                        response.push_str(
                                            "--------------------------------|------|----|
\r\n",
                                        );

                                        for ap in aps {
                                            // Pad SSID to 31 chars
                                            let ssid = ap.ssid.as_str();
                                            let ssid_padded = if ssid.len() > 31 {
                                                &ssid[..31]
                                            } else {
                                                ssid
                                            };

                                            response.push_str(&format!(
                                                "{:31} | {:4} | {:2}\r\n",
                                                ssid_padded, ap.signal_strength, ap.channel
                                            ));
                                        }
                                    }
                                }
                                Err(e) => {
                                    response.push_str(&format!("\r\nScan failed: {:?}", e));
                                }
                            }
                        }
                        Err(_) => {
                            response.push_str("WiFi manager lock error");
                        }
                    }
                } else {
                    response.push_str("WiFi not initialized");
                }
            }
            // MQTT Commands
            CliCommand::MqttStatus => {
                log::info!("CLI: MQTT status requested");
                if let Some(ref mqtt) = self.mqtt {
                    let status = mqtt.get_status();
                    let connected = mqtt.is_connected();

                    response.push_str("MQTT Status:\r\n");
                    response.push_str(&format!(
                        "  Connection: {}\r\n",
                        if connected {
                            "✅ Connected"
                        } else {
                            "❌ Disconnected"
                        }
                    ));
                    response.push_str(&format!("  Broker: {}\r\n", status.broker_url));
                    response.push_str(&format!("  Client ID: {}\r\n", status.client_id));
                    response.push_str(&format!(
                        "  Publishing: {}\r\n",
                        if self.mqtt_publishing_enabled {
                            "✅ Enabled"
                        } else {
                            "❌ Disabled"
                        }
                    ));

                    let subs = status.subscriptions.lock().unwrap();
                    response.push_str(&format!("  Subscriptions ({}):\r\n", subs.len()));
                    for sub in subs.iter() {
                        response.push_str(&format!("    - {}\r\n", sub));
                    }

                    let pub_count = *status.publish_count.lock().unwrap();
                    let recv_count = *status.receive_count.lock().unwrap();
                    response.push_str(&format!("  Published: {} messages\r\n", pub_count));
                    response.push_str(&format!("  Received: {} messages\r\n", recv_count));

                    let last_pub = status.last_published_topic.lock().unwrap();
                    if !last_pub.is_empty() {
                        response.push_str(&format!("  Last published: {}\r\n", last_pub));
                    }

                    let last_recv_topic = status.last_received_topic.lock().unwrap();
                    let last_recv_msg = status.last_received_message.lock().unwrap();
                    if !last_recv_topic.is_empty() {
                        response.push_str(&format!(
                            "  Last received: {} = {}",
                            last_recv_topic, last_recv_msg
                        ));
                    }
                } else {
                    response.push_str("MQTT Status: Not initialized");
                }
            }
            CliCommand::MqttPublish(topic, message) => {
                log::info!("CLI: MQTT publish requested to topic: {}", topic);
                if let Some(ref mqtt) = self.mqtt {
                    use esp_idf_svc::mqtt::client::QoS;
                    match mqtt.publish(&topic, message.as_bytes(), QoS::AtLeastOnce, false) {
                        Ok(_) => {
                            response.push_str(&format!("Published to {}: {}", topic, message));
                        }
                        Err(e) => {
                            response.push_str(&format!("MQTT publish failed: {:?}", e));
                        }
                    }
                } else {
                    response.push_str("MQTT not initialized");
                }
            }
            CliCommand::MqttEnable => {
                log::info!("CLI: MQTT publishing enable requested");
                if self.mqtt.is_some() {
                    self.mqtt_publishing_enabled = true;
                    response.push_str("MQTT publishing enabled");
                } else {
                    response.push_str("MQTT not initialized - cannot enable publishing");
                }
            }
            CliCommand::MqttDisable => {
                log::info!("CLI: MQTT publishing disable requested");
                self.mqtt_publishing_enabled = false;
                response.push_str("MQTT publishing disabled");
            }
            CliCommand::InvalidSyntax(msg) => {
                log::info!("CLI: Invalid command syntax: {}", msg);

                response.push_str("Invalid Command Syntax: ");
                response.push_str(&msg);

                // Publish error event to MQTT (if enabled)
                if self.mqtt_publishing_enabled {
                    if let Some(ref mqtt) = self.mqtt {
                        let error_json = format!(
                            r#"{{"timestamp":{},"source":"cli","event":"invalid_syntax","error":"{}"}}"#,
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                            msg.replace('"', "'")
                        );

                        if let Some(ref topic) = self.event_error_topic {
                            let _ = mqtt.publish(topic, error_json.as_bytes(), esp_idf_svc::mqtt::client::QoS::AtMostOnce, false);
                        }
                    }
                }
            }
            CliCommand::Unknown(cmd) => {
                log::info!("CLI: Unknown command: {}", cmd);

                response.push_str("Unknown Command: ");
                response.push_str(&cmd);
                response.push_str(". Type 'help' for available commands.");

                // Publish error event to MQTT (if enabled)
                if self.mqtt_publishing_enabled {
                    if let Some(ref mqtt) = self.mqtt {
                        let error_json = format!(
                            r#"{{"timestamp":{},"source":"cli","event":"unknown_command","error":"{}"}}"#,
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                            cmd.replace('"', "'")
                        );

                        if let Some(ref topic) = self.event_error_topic {
                            let _ = mqtt.publish(topic, error_json.as_bytes(), esp_idf_svc::mqtt::client::QoS::AtMostOnce, false);
                        }
                    }
                }
            }
        }

        Ok(response)
    }
}
