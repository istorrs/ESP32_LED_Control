use esp32_led_flasher::cli::{CommandHandler, CommandParser, Terminal};
use esp32_led_flasher::led::{LedManager, PulseConfig};
use esp32_led_flasher::mqtt::MqttClient;
use esp32_led_flasher::wifi::WifiManager;
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{Output, PinDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::uart::{config::Config as UartConfig, UartDriver};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::mqtt::client::QoS;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Get ESP32 base MAC address (chip ID) as a hex string
fn get_chip_id() -> String {
    let mut mac = [0u8; 6];
    unsafe {
        sys::esp_efuse_mac_get_default(mac.as_mut_ptr());
    }
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

fn main() -> anyhow::Result<()> {
    // Initialize ESP-IDF system services
    sys::link_patches();

    // Initialize logging
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("ESP32 LED Flasher with WiFi and MQTT Control");
    log::info!("Initializing...");

    let peripherals = Peripherals::take()?;

    log::info!("✅ ESP32 initialized with ESP-IDF");

    // Get unique chip ID for device-specific MQTT topics
    let chip_id = get_chip_id();
    log::info!("📟 Chip ID: {}", chip_id);

    // Initialize system event loop and NVS for WiFi
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // WiFi Configuration
    const WIFI_SSID: &str = "Ian Storrs 1";
    const WIFI_PASSWORD: &str = "abbaabba";

    // MQTT Configuration - Mosquitto public test broker
    const MQTT_BROKER: &str = "mqtt://test.mosquitto.org:1883";
    const MQTT_STATUS_TOPIC: &str = "istorrs/led/status";
    const MQTT_CONTROL_TOPIC_SHARED: &str = "istorrs/led/control"; // Shared topic for broadcast commands

    // Device-specific MQTT topics based on chip ID
    let mqtt_client_id = format!("esp32-led-{}", chip_id.replace(":", ""));
    let mqtt_control_topic_device = format!("istorrs/led/{}/control", chip_id);
    let mqtt_status_topic_device = format!("istorrs/led/{}/status", chip_id);

    log::info!("📡 MQTT Client ID: {}", mqtt_client_id);
    log::info!("📡 MQTT Topics:");
    log::info!("   Control (shared): {}", MQTT_CONTROL_TOPIC_SHARED);
    log::info!("   Control (device): {}", mqtt_control_topic_device);
    log::info!("   Status (device):  {}", mqtt_status_topic_device);

    // Initialize LED on GPIO2 (built-in LED) with default pulse
    log::info!("💡 Initializing LED on GPIO2 with default pulse (500ms / 5s)...");
    let led_pin = PinDriver::output(peripherals.pins.gpio2)?;
    let led_manager = Arc::new(LedManager::new(led_pin));
    log::info!("✅ LED initialized and pulsing");

    // Initialize WiFi and connect immediately (always-on mode)
    let wifi = if WIFI_SSID != "YOUR_SSID" {
        log::info!("🌐 Connecting to WiFi...");
        log::info!("  SSID: {}", WIFI_SSID);

        match WifiManager::new(
            peripherals.modem,
            sysloop.clone(),
            nvs.clone(),
            WIFI_SSID,
            WIFI_PASSWORD,
        ) {
            Ok(wifi) => {
                log::info!("✅ WiFi connected successfully");
                if let Ok(ip) = wifi.get_ip() {
                    log::info!("📡 IP Address: {}", ip);
                }
                Some(Arc::new(Mutex::new(wifi)))
            }
            Err(e) => {
                log::error!("❌ WiFi connection failed: {:?}", e);
                log::warn!("⚠️  Continuing without WiFi - LED control will be CLI-only");
                None
            }
        }
    } else {
        log::info!("⏭️  WiFi not configured (update WIFI_SSID in main.rs)");
        None
    };

    // Initialize MQTT and connect immediately (always-on mode)
    let mqtt = if wifi.is_some() {
        log::info!("📡 Connecting to MQTT broker: {}", MQTT_BROKER);

        match MqttClient::new(MQTT_BROKER, &mqtt_client_id) {
            Ok(mut mqtt_client) => {
                log::info!("✅ MQTT connected successfully");

                // Subscribe to LED control topics
                log::info!("📬 Subscribing to control topics...");
                if let Err(e) = mqtt_client.subscribe(&mqtt_control_topic_shared, QoS::AtMostOnce)
                {
                    log::warn!("⚠️  Failed to subscribe to shared control topic: {:?}", e);
                } else {
                    log::info!("  ✅ Subscribed to: {}", mqtt_control_topic_shared);
                }

                if let Err(e) =
                    mqtt_client.subscribe(&mqtt_control_topic_device, QoS::AtLeastOnce)
                {
                    log::warn!("⚠️  Failed to subscribe to device control topic: {:?}", e);
                } else {
                    log::info!("  ✅ Subscribed to: {}", mqtt_control_topic_device);
                }

                // Set up MQTT message handler for LED control
                let led_for_mqtt = led_manager.clone();
                mqtt_client.set_message_callback(move |topic, payload| {
                    log::info!("📨 MQTT message received on {}: {}", topic, payload);

                    // Parse JSON payload for LED control
                    // Expected format: {"duration_ms": 500, "period_ms": 5000}
                    // Or simple commands: "on", "off", "blink"
                    match payload.as_str() {
                        "on" => {
                            log::info!("💡 MQTT command: Turn LED ON");
                            led_for_mqtt.turn_on();
                        }
                        "off" => {
                            log::info!("💡 MQTT command: Turn LED OFF");
                            led_for_mqtt.turn_off();
                        }
                        _ => {
                            // Try to parse as JSON
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(payload) {
                                if let (Some(duration), Some(period)) = (
                                    json.get("duration_ms").and_then(|v| v.as_u64()),
                                    json.get("period_ms").and_then(|v| v.as_u64()),
                                ) {
                                    let duration_ms = duration as u32;
                                    let period_ms = period as u32;

                                    log::info!(
                                        "💡 MQTT command: Set pulse {}ms / {}ms",
                                        duration_ms,
                                        period_ms
                                    );

                                    match PulseConfig::new(duration_ms, period_ms) {
                                        Ok(config) => {
                                            led_for_mqtt.set_pulse(config);
                                        }
                                        Err(e) => {
                                            log::warn!("⚠️  Invalid pulse config: {}", e);
                                        }
                                    }
                                } else {
                                    log::warn!("⚠️  Invalid JSON format - expected duration_ms and period_ms");
                                }
                            } else {
                                log::warn!("⚠️  Unknown MQTT command: {}", payload);
                            }
                        }
                    }
                });

                // Publish initial status
                let status = format!(
                    r#"{{"state":"pulsing","duration_ms":500,"period_ms":5000,"device_id":"{}"}}"#,
                    chip_id
                );
                if let Err(e) = mqtt_client.publish(
                    &mqtt_status_topic_device,
                    status.as_bytes(),
                    QoS::AtLeastOnce,
                    false,
                ) {
                    log::warn!("⚠️  Failed to publish initial status: {:?}", e);
                } else {
                    log::info!("📤 Published initial LED status");
                }

                Some(Arc::new(mqtt_client))
            }
            Err(e) => {
                log::error!("❌ MQTT connection failed: {:?}", e);
                log::warn!("⚠️  Continuing without MQTT - LED control will be CLI-only");
                None
            }
        }
    } else {
        log::info!("⏭️  MQTT not initialized (WiFi not available)");
        None
    };

    // Initialize UART0 for CLI (USB-C on most ESP32 boards)
    log::info!("📟 Initializing UART0 for CLI...");
    let uart_config = UartConfig::default().baudrate(esp_idf_hal::units::Hertz(115200));
    let uart = UartDriver::new(
        peripherals.uart0,
        peripherals.pins.gpio1,  // TX
        peripherals.pins.gpio3,  // RX
        Option::<esp_idf_hal::gpio::Gpio0>::None,
        Option::<esp_idf_hal::gpio::Gpio0>::None,
        &uart_config,
    )?;

    log::info!("✅ UART0 initialized at 115200 baud");

    // Create terminal and command handler
    let mut terminal = Terminal::new(uart);
    let mut command_handler = CommandHandler::new().with_led(led_manager.clone());

    if let Some(wifi_ref) = &wifi {
        command_handler = command_handler.with_wifi(wifi_ref.clone());
    }
    if let Some(mqtt_ref) = &mqtt {
        command_handler = command_handler.with_mqtt(mqtt_ref.clone());
    }

    // Display welcome banner
    terminal.write_line("\r\n")?;
    terminal.write_line("╔═══════════════════════════════════════════════════════╗")?;
    terminal.write_line("║       ESP32 LED Flasher with WiFi/MQTT Control       ║")?;
    terminal.write_line("╚═══════════════════════════════════════════════════════╝")?;
    terminal.write_line(&format!("  Chip ID: {}", chip_id))?;
    terminal.write_line("  Default pulse: 500ms ON / 5s period")?;
    terminal.write_line("")?;

    if wifi.is_some() {
        terminal.write_line("  WiFi: ✅ Connected")?;
    } else {
        terminal.write_line("  WiFi: ❌ Not connected")?;
    }

    if mqtt.is_some() {
        terminal.write_line("  MQTT: ✅ Connected")?;
        terminal.write_line(&format!("  Control topics:"))?;
        terminal.write_line(&format!("    {}", mqtt_control_topic_device))?;
        terminal.write_line(&format!("    {}", MQTT_CONTROL_TOPIC_SHARED))?;
    } else {
        terminal.write_line("  MQTT: ❌ Not connected")?;
    }

    terminal.write_line("")?;
    terminal.write_line("Type 'help' for available commands")?;
    terminal.write_line("")?;

    // Main CLI loop
    log::info!("🚀 Entering main CLI loop");

    // Spawn background task for periodic MQTT status publishing
    if let (Some(mqtt_ref), Some(_wifi_ref)) = (mqtt.clone(), wifi.clone()) {
        let led_for_status = led_manager.clone();
        let status_topic = mqtt_status_topic_device.clone();
        let chip_id_for_status = chip_id.clone();

        std::thread::Builder::new()
            .stack_size(8192)
            .name("mqtt_status".to_string())
            .spawn(move || {
                log::info!("📊 MQTT status publisher started (every 60s)");
                loop {
                    FreeRtos::delay_ms(60_000); // Publish every 60 seconds

                    let status = led_for_status.get_status();
                    let status_json = match status {
                        esp32_led_flasher::led::LedStatus::Off => {
                            format!(
                                r#"{{"state":"off","device_id":"{}"}}"#,
                                chip_id_for_status
                            )
                        }
                        esp32_led_flasher::led::LedStatus::SolidOn => {
                            format!(
                                r#"{{"state":"on","device_id":"{}"}}"#,
                                chip_id_for_status
                            )
                        }
                        esp32_led_flasher::led::LedStatus::CustomPulse(config) => {
                            format!(
                                r#"{{"state":"pulsing","duration_ms":{},"period_ms":{},"device_id":"{}"}}"#,
                                config.duration_ms, config.period_ms, chip_id_for_status
                            )
                        }
                        esp32_led_flasher::led::LedStatus::SlowBlink => {
                            format!(
                                r#"{{"state":"blink","frequency_hz":1,"device_id":"{}"}}"#,
                                chip_id_for_status
                            )
                        }
                        esp32_led_flasher::led::LedStatus::FastBlink => {
                            format!(
                                r#"{{"state":"blink","frequency_hz":5,"device_id":"{}"}}"#,
                                chip_id_for_status
                            )
                        }
                    };

                    if let Err(e) = mqtt_ref.publish(
                        &status_topic,
                        status_json.as_bytes(),
                        QoS::AtMostOnce,
                        false,
                    ) {
                        log::warn!("⚠️  Failed to publish status: {:?}", e);
                    } else {
                        log::debug!("📤 Published LED status");
                    }
                }
            })
            .expect("Failed to spawn MQTT status publisher");
    }

    loop {
        if let Ok(command) = terminal.read_command(&mut command_handler) {
            match command_handler.execute_command(command) {
                Ok(response) => {
                    if !response.is_empty() {
                        let _ = terminal.write_line(&response);
                    }
                }
                Err(e) => {
                    let _ = terminal.write_line(&format!("Error: {}", e));
                }
            }
        }

        // Small delay to prevent busy loop
        FreeRtos::delay_ms(10);
    }
}
