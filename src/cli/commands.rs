use super::{CliCommand, CliError};
use crate::mqtt::MqttClient;
use crate::mtu::{GpioMtuTimerV2, MtuCommand};
use crate::wifi::WifiManager;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct CommandHandler {
    start_time: Instant,
    mtu: Option<Arc<GpioMtuTimerV2>>,
    mtu_cmd_sender: Option<Sender<MtuCommand>>,
    wifi: Option<Arc<Mutex<WifiManager>>>,
    mqtt: Option<Arc<MqttClient>>,
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
            mtu: None,
            mtu_cmd_sender: None,
            wifi: None,
            mqtt: None,
        }
    }

    pub fn with_mtu(mut self, mtu: Arc<GpioMtuTimerV2>, cmd_sender: Sender<MtuCommand>) -> Self {
        self.mtu = Some(mtu);
        self.mtu_cmd_sender = Some(cmd_sender);
        self
    }

    pub fn with_wifi(mut self, wifi: Arc<Mutex<WifiManager>>) -> Self {
        self.wifi = Some(wifi);
        self
    }

    pub fn with_mqtt(mut self, mqtt: Arc<MqttClient>) -> Self {
        self.mqtt = Some(mqtt);
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
                response.push_str("ESP32 Water Meter MTU Interface v1.0.0\r\n");
                response.push_str("Built with ESP-IDF");
            }
            CliCommand::Status => {
                log::info!("CLI: Status requested");
                response.push_str("System Status:\r\n");
                response.push_str("  Firmware: ESP32 Water Meter MTU v1.0.0\r\n");
                response.push_str("  Platform: ESP32 with ESP-IDF\r\n");
                response.push_str("  MTU: GPIO4 (clock), GPIO5 (data)\r\n");
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
            CliCommand::MtuStart(duration) => {
                log::info!("CLI: MTU start requested");
                if let Some(ref sender) = self.mtu_cmd_sender {
                    if let Some(ref mtu) = self.mtu {
                        let duration_secs = duration.unwrap_or(30);

                        if mtu.is_running() {
                            response.push_str("MTU is already running. Use 'mtu_stop' first.");
                        } else {
                            // Send start command to MTU thread
                            match sender.send(MtuCommand::Start {
                                duration_secs: duration_secs.into(),
                            }) {
                                Ok(_) => {
                                    response.push_str(&format!(
                                        "MTU operation started for {} seconds",
                                        duration_secs
                                    ));
                                }
                                Err(_) => {
                                    response
                                        .push_str("Error: Failed to send command to MTU thread");
                                }
                            }
                        }
                    } else {
                        response.push_str("MTU not configured");
                    }
                } else {
                    response.push_str("MTU not configured");
                }
            }
            CliCommand::MtuStop => {
                log::info!("CLI: MTU stop requested");
                if let Some(ref sender) = self.mtu_cmd_sender {
                    if let Some(ref mtu) = self.mtu {
                        if mtu.is_running() {
                            // Send stop command to MTU thread
                            match sender.send(MtuCommand::Stop) {
                                Ok(_) => {
                                    response.push_str("MTU stop signal sent");
                                }
                                Err(_) => {
                                    response
                                        .push_str("Error: Failed to send command to MTU thread");
                                }
                            }
                        } else {
                            response.push_str("MTU is not running");
                        }
                    } else {
                        response.push_str("MTU not configured");
                    }
                } else {
                    response.push_str("MTU not configured");
                }
            }
            CliCommand::MtuStatus => {
                log::info!("CLI: MTU status requested");
                if let Some(ref mtu) = self.mtu {
                    let baud_rate = mtu.get_baud_rate();
                    let uart_format = mtu.get_uart_format();
                    let (successful, corrupted, cycles) = mtu.get_stats();
                    let total_reads = successful + corrupted;

                    response.push_str("MTU Status:\r\n");
                    response.push_str(&format!(
                        "  State: {}\r\n",
                        if mtu.is_running() {
                            "Running"
                        } else {
                            "Stopped"
                        }
                    ));
                    response.push_str(&format!("  Baud rate: {} bps\r\n", baud_rate));
                    response.push_str(&format!("  UART format: {}\r\n", uart_format.as_str()));
                    response.push_str("  Pins: GPIO4 (clock), GPIO5 (data)\r\n");
                    response.push_str(&format!("  Total cycles: {}\r\n", cycles));
                    response.push_str("  Statistics:\r\n");
                    response.push_str(&format!("    Successful reads: {}\r\n", successful));
                    response.push_str(&format!("    Corrupted reads: {}\r\n", corrupted));

                    if total_reads > 0 {
                        let success_rate = (successful as f32 / total_reads as f32) * 100.0;
                        response.push_str(&format!("    Success rate: {:.1}%\r\n", success_rate));
                    }

                    if let Some(last_msg) = mtu.get_last_message() {
                        response.push_str(&format!("  Last message: {}", last_msg.as_str()));
                    } else {
                        response.push_str("  Last message: None");
                    }
                } else {
                    response.push_str("MTU not configured");
                }
            }
            CliCommand::MtuBaud(baud_rate) => {
                log::info!("CLI: MTU baud rate set to {}", baud_rate);
                if let Some(ref mtu) = self.mtu {
                    if mtu.is_running() {
                        response.push_str("Cannot change baud rate while MTU is running.\r\n");
                        response.push_str("Use 'mtu_stop' first.");
                    } else {
                        mtu.set_baud_rate(baud_rate);
                        response.push_str(&format!("MTU baud rate set to {} bps", baud_rate));
                    }
                } else {
                    response.push_str("MTU not configured");
                }
            }
            CliCommand::MtuFormat(format_str) => {
                log::info!("CLI: MTU UART format set to {}", format_str);
                if let Some(ref sender) = self.mtu_cmd_sender {
                    if let Some(ref mtu) = self.mtu {
                        if mtu.is_running() {
                            response
                                .push_str("Cannot change UART format while MTU is running.\r\n");
                            response.push_str("Use 'mtu_stop' first.");
                        } else if let Ok(format) =
                            format_str.parse::<crate::uart_format::UartFormat>()
                        {
                            match sender.send(MtuCommand::SetUartFormat { format }) {
                                Ok(_) => {
                                    response.push_str(&format!(
                                        "MTU UART format set to {}",
                                        format_str
                                    ));
                                }
                                Err(_) => {
                                    response
                                        .push_str("Error: Failed to send command to MTU thread");
                                }
                            }
                        } else {
                            response.push_str("Invalid UART format");
                        }
                    } else {
                        response.push_str("MTU not configured");
                    }
                } else {
                    response.push_str("MTU not configured");
                }
            }
            CliCommand::MtuReset => {
                log::info!("CLI: MTU statistics reset requested");
                if let Some(ref mtu) = self.mtu {
                    mtu.reset_stats();
                    response.push_str("MTU statistics reset");
                } else {
                    response.push_str("MTU not configured");
                }
            }
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
                                        response.push_str(
                                            "SSID                    RSSI  Ch  Security\r\n",
                                        );
                                        response.push_str(
                                            "──────────────────────  ────  ──  ────────\r\n",
                                        );

                                        // Sort by signal strength (highest first)
                                        let mut sorted_aps = aps;
                                        sorted_aps.sort_by(|a, b| {
                                            b.signal_strength.cmp(&a.signal_strength)
                                        });

                                        for ap in sorted_aps.iter().take(20) {
                                            let ssid = if ap.ssid.is_empty() {
                                                "<hidden>".to_string()
                                            } else {
                                                ap.ssid.to_string()
                                            };

                                            let auth = match ap.auth_method {
                                                Some(esp_idf_svc::wifi::AuthMethod::None) => "Open",
                                                Some(esp_idf_svc::wifi::AuthMethod::WEP) => "WEP",
                                                Some(esp_idf_svc::wifi::AuthMethod::WPA) => "WPA",
                                                Some(
                                                    esp_idf_svc::wifi::AuthMethod::WPA2Personal,
                                                ) => "WPA2",
                                                Some(
                                                    esp_idf_svc::wifi::AuthMethod::WPAWPA2Personal,
                                                ) => "WPA/WPA2",
                                                Some(
                                                    esp_idf_svc::wifi::AuthMethod::WPA2Enterprise,
                                                ) => "WPA2-Ent",
                                                Some(
                                                    esp_idf_svc::wifi::AuthMethod::WPA3Personal,
                                                ) => "WPA3",
                                                Some(
                                                    esp_idf_svc::wifi::AuthMethod::WPA2WPA3Personal,
                                                ) => "WPA2/WPA3",
                                                None => "Unknown",
                                                _ => "Other",
                                            };

                                            response.push_str(&format!(
                                                "{:<22}  {:>4}  {:>2}  {}\r\n",
                                                if ssid.len() > 22 {
                                                    format!("{}...", &ssid[..19])
                                                } else {
                                                    ssid
                                                },
                                                ap.signal_strength,
                                                ap.channel,
                                                auth
                                            ));
                                        }

                                        if sorted_aps.len() > 20 {
                                            response.push_str(&format!(
                                                "\r\n(Showing top 20 of {} networks)",
                                                sorted_aps.len()
                                            ));
                                        }
                                    }
                                }
                                Err(e) => {
                                    response.push_str(&format!("WiFi scan failed: {:?}", e));
                                }
                            }
                        }
                        Err(_) => {
                            response.push_str("WiFi scan failed: Lock error");
                        }
                    }
                } else {
                    response.push_str("WiFi scan failed: Not initialized");
                }
            }
            CliCommand::MqttConnect(_broker_url) => {
                log::info!("CLI: MQTT connect requested");
                response
                    .push_str("MQTT connect not available - MQTT must be initialized at startup");
            }
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
