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

    pub fn execute_command(&mut self, command: CliCommand) -> Result<String, CliError> {
        let mut response = String::new();

        match command {
            CliCommand::Empty => {
                // Empty command - just return empty response (no error)
            }
            CliCommand::Help => {
                // Help is handled in terminal.rs
                response.push_str("Help displayed");
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
                } else {
                    response.push_str("LED not initialized");
                }
            }
            CliCommand::LedOff => {
                log::info!("CLI: LED off requested");
                if let Some(ref led) = self.led {
                    led.turn_off();
                    response.push_str("LED turned OFF");
                } else {
                    response.push_str("LED not initialized");
                }
            }
            CliCommand::LedPulse(duration_ms, period_ms) => {
                log::info!(
                    "CLI: LED pulse requested - duration: {}ms, period: {}ms",
                    duration_ms,
                    period_ms
                );
                if let Some(ref led) = self.led {
                    match PulseConfig::new(duration_ms, period_ms) {
                        Ok(config) => {
                            led.set_pulse(config);
                            response.push_str(&format!(
                                "LED pulse set: {}ms ON / {}ms period",
                                duration_ms, period_ms
                            ));
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
                                "  State: Pulsing\r\n  Duration: {}ms\r\n  Period: {}ms",
                                config.duration_ms, config.period_ms
                            ));
                            let duty_cycle =
                                (config.duration_ms as f32 / config.period_ms as f32) * 100.0;
                            response.push_str(&format!("\r\n  Duty cycle: {:.1}%", duty_cycle));
                        }
                        LedStatus::SlowBlink => {
                            response.push_str("  State: Slow blink (1 Hz)");
                        }
                        LedStatus::FastBlink => {
                            response.push_str("  State: Fast blink (5 Hz)");
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
                                            "SSID                            | RSSI | Ch | Sec\r\n",
                                        );
                                        response.push_str(
                                            "--------------------------------|------|----|---------\r\n",
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
                                                "{:31} | {:4} | {:2} | {:?}\r\n",
                                                ssid_padded, ap.signal_strength, ap.channel, ap.auth
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
            CliCommand::Unknown(cmd) => {
                log::info!("CLI: Unknown command: {}", cmd);
                response.push_str("Unknown command: ");
                response.push_str(&cmd);
                response.push_str(". Type 'help' for available commands.");
            }
        }

        Ok(response)
    }
}
