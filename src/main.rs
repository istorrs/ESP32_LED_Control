use esp32_led_flasher::cli::{CommandHandler, CommandParser, Terminal};
use esp32_led_flasher::tcp_cli;
use esp32_led_flasher::led::LedManager;
#[allow(unused_imports)] // Used when MQTT is enabled
use esp32_led_flasher::led::PulseConfig;
#[allow(unused_imports)] // Used when MQTT is enabled
use esp32_led_flasher::mqtt::MqttClient;
use esp32_led_flasher::wifi::WifiManager;
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::uart::{config::Config as UartConfig, UartDriver};
use esp_idf_svc::eventloop::EspSystemEventLoop;
#[allow(unused_imports)] // Used when MQTT is enabled
use esp_idf_svc::mqtt::client::QoS;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;
#[allow(unused_imports)] // Used when MQTT is enabled
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

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

    // Suppress noisy MQTT connection error logs from ESP-IDF components
    // These components spam errors during connection retries when broker is down
    unsafe {
        sys::esp_log_level_set(
            b"esp-tls\0".as_ptr() as *const std::os::raw::c_char,
            sys::esp_log_level_t_ESP_LOG_WARN,
        );
        sys::esp_log_level_set(
            b"transport_base\0".as_ptr() as *const std::os::raw::c_char,
            sys::esp_log_level_t_ESP_LOG_WARN,
        );
        sys::esp_log_level_set(
            b"mqtt_client\0".as_ptr() as *const std::os::raw::c_char,
            sys::esp_log_level_t_ESP_LOG_WARN,
        );
    }

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

    // MQTT Configuration - Flespi broker with token authentication
    #[allow(dead_code)] // Used when MQTT is enabled
    const MQTT_BROKER: &str = "mqtt://mqtt.flespi.io:1883";
    #[allow(dead_code)] // Used when MQTT is enabled
    const MQTT_USERNAME: &str = "FlespiToken vQHE4KM46e7Npgu8EgFGzikViRjvLUdSXYIoFX9W0ECFhBouPuxRHxfOHXzUN2lb";
    #[allow(dead_code)] // Used when MQTT is enabled
    const MQTT_PASSWORD: &str = "";
    const MQTT_CONTROL_TOPIC_SHARED: &str = "istorrs/led/control"; // Shared topic for broadcast commands

    // Device-specific MQTT topics based on chip ID
    let mqtt_client_id = format!("esp32-led-{}", chip_id.replace(":", ""));
    let mqtt_control_topic_device = format!("istorrs/led/{}/control", chip_id);
    let mqtt_status_topic_device = format!("istorrs/led/{}/status", chip_id);

    log::info!("📡 MQTT Configuration (disabled by default):");
    log::info!("   Client ID: {}", mqtt_client_id);
    log::info!("   Control (shared): {}", MQTT_CONTROL_TOPIC_SHARED);
    log::info!("   Control (device): {}", mqtt_control_topic_device);
    log::info!("   Status (device):  {}", mqtt_status_topic_device);
    log::info!("   Use 'mqtt_enable' command to connect");

    // Initialize LED on GPIO2 (built-in LED) with LEDC for PWM control and hardware timer for microsecond precision
    log::info!("💡 Initializing LED on GPIO2 with LEDC + hardware timer (500ms / 5s @ 75%)...");
    let led_manager = Arc::new(
        LedManager::new(
            peripherals.ledc.channel0,
            peripherals.ledc.timer0,
            peripherals.pins.gpio2,
            peripherals.timer00, // Hardware timer for microsecond precision
        )?
    );
    log::info!("✅ LED initialized with hardware timer (1μs resolution)");

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
                    log::info!("🌐 TCP CLI will be available at {}:{}", ip, tcp_cli::TCP_CLI_PORT);
                }

                // Start TCP CLI server — same CLI as serial, accessible over WiFi
                log::info!("📟 Starting TCP CLI server on port {}...", tcp_cli::TCP_CLI_PORT);
                match tcp_cli::start(led_manager.clone()) {
                    Ok(()) => log::info!("✅ TCP CLI server started"),
                    Err(e) => log::warn!("⚠️  TCP CLI server failed to start: {:?}", e),
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

    // MQTT is disabled by default - use mqtt_enable command to enable publishing
    // To enable MQTT at startup, uncomment the following code
    let mqtt: Option<Arc<MqttClient>> = None;

    /*
    // Initialize MQTT and connect immediately (always-on mode)
    let mqtt = if wifi.is_some() {
        log::info!("📡 Connecting to MQTT broker: {}", MQTT_BROKER);

        // Create a channel for STATUS_REQ messages
        let (status_req_tx, status_req_rx) = mpsc::channel::<()>();

        // Set up MQTT message handler for provisioning and control
        let led_for_mqtt = led_manager.clone();
        let message_callback = Arc::new(move |topic: &str, payload: &[u8]| {
            if let Ok(payload_str) = std::str::from_utf8(payload) {
                log::info!("📨 MQTT message on '{}': {}", topic, payload_str);

                // Handle STATUS_REQ - send signal to status responder thread
                if payload_str == "STATUS_REQ" {
                    log::info!("📊 Status request received");
                    let _ = status_req_tx.send(()); // Non-blocking send
                    return;
                }

                // Provisioning: Parse JSON payload for LED config changes
                // Expected format: {"duration_ms": 500, "period_ms": 5000, "brightness_percent": 75}
                // Or simple commands: "on", "off"
                match payload_str {
                    "on" => {
                        log::info!("💡 MQTT provisioning: Turn LED ON");
                        led_for_mqtt.turn_on();
                    }
                    "off" => {
                        log::info!("💡 MQTT provisioning: Turn LED OFF");
                        led_for_mqtt.turn_off();
                    }
                    _ => {
                        // Try to parse as JSON for pulse configuration
                        // Support both microseconds (duration_us/period_us) and milliseconds (duration_ms/period_ms) for backward compatibility
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(payload_str) {
                            // Try microseconds first
                            let (duration_us, period_us) = if let (Some(duration), Some(period)) = (
                                json.get("duration_us").and_then(|v| v.as_u64()),
                                json.get("period_us").and_then(|v| v.as_u64()),
                            ) {
                                (duration as u32, period as u32)
                            } else if let (Some(duration), Some(period)) = (
                                json.get("duration_ms").and_then(|v| v.as_u64()),
                                json.get("period_ms").and_then(|v| v.as_u64()),
                            ) {
                                // Convert milliseconds to microseconds
                                (duration as u32 * 1000, period as u32 * 1000)
                            } else {
                                log::warn!("⚠️  Invalid JSON format - expected duration_us/period_us or duration_ms/period_ms");
                                return;
                            };

                            // Brightness is optional, defaults to 75%
                            let brightness_percent = json.get("brightness_percent")
                                .and_then(|v| v.as_u64())
                                .map(|v| v as u8)
                                .unwrap_or(75);

                            log::info!(
                                "💡 MQTT provisioning: Set pulse {}μs / {}μs @ {}%",
                                duration_us,
                                period_us,
                                brightness_percent
                            );

                            match PulseConfig::new(duration_us, period_us, brightness_percent) {
                                Ok(config) => {
                                    led_for_mqtt.set_pulse(config);
                                }
                                Err(e) => {
                                    log::warn!("⚠️  Invalid pulse config: {}", e);
                                }
                            }
                        } else {
                            log::warn!("⚠️  Unknown MQTT command: {}", payload_str);
                        }
                    }
                }
            }
        });

        match MqttClient::new(
            MQTT_BROKER,
            &mqtt_client_id,
            Some(MQTT_USERNAME),
            Some(MQTT_PASSWORD),
            message_callback,
        ) {
            Ok(mqtt_client) => {
                log::info!("✅ MQTT connected successfully");

                // Subscribe to LED control topics
                log::info!("📬 Subscribing to control topics...");
                if let Err(e) = mqtt_client.subscribe(MQTT_CONTROL_TOPIC_SHARED, QoS::AtMostOnce)
                {
                    log::warn!("⚠️  Failed to subscribe to shared control topic: {:?}", e);
                } else {
                    log::info!("  ✅ Subscribed to: {}", MQTT_CONTROL_TOPIC_SHARED);
                }

                if let Err(e) =
                    mqtt_client.subscribe(&mqtt_control_topic_device, QoS::AtLeastOnce)
                {
                    log::warn!("⚠️  Failed to subscribe to device control topic: {:?}", e);
                } else {
                    log::info!("  ✅ Subscribed to: {}", mqtt_control_topic_device);
                }

                log::info!("💡 MQTT ready for provisioning - send STATUS_REQ to get device status");

                let mqtt_client_arc = Arc::new(mqtt_client);

                // Spawn status responder thread to handle STATUS_REQ messages
                let mqtt_for_status = mqtt_client_arc.clone();
                let led_for_status = led_manager.clone();
                let status_topic = mqtt_status_topic_device.clone();
                let chip_id_for_status = chip_id.clone();
                let wifi_ssid = WIFI_SSID.to_string();

                std::thread::Builder::new()
                    .stack_size(8192)
                    .name("mqtt_status_resp".to_string())
                    .spawn(move || {
                        log::info!("📊 MQTT status responder thread started");

                        // Wait for STATUS_REQ messages from the channel
                        while let Ok(()) = status_req_rx.recv() {
                            log::info!("📊 Processing STATUS_REQ, gathering device info...");

                            // Gather device status
                            let led_status = led_for_status.get_status();
                            let stats = led_for_status.get_statistics();

                            // Build status JSON
                            let status_json = match led_status {
                                esp32_led_flasher::led::LedStatus::Off => {
                                    format!(
                                        r#"{{"device_id":"{}","state":"off","wifi_ssid":"{}","pulse_count":{},"config_age_secs":{:.1}}}"#,
                                        chip_id_for_status,
                                        wifi_ssid,
                                        stats.pulse_count,
                                        stats.config_changed_time.elapsed().as_secs_f32()
                                    )
                                }
                                esp32_led_flasher::led::LedStatus::SolidOn => {
                                    format!(
                                        r#"{{"device_id":"{}","state":"on","wifi_ssid":"{}","pulse_count":{},"config_age_secs":{:.1}}}"#,
                                        chip_id_for_status,
                                        wifi_ssid,
                                        stats.pulse_count,
                                        stats.config_changed_time.elapsed().as_secs_f32()
                                    )
                                }
                                esp32_led_flasher::led::LedStatus::CustomPulse(config) => {
                                    let last_pulse_secs = stats.last_pulse_time
                                        .map(|t| t.elapsed().as_secs_f32())
                                        .unwrap_or(-1.0);
                                    format!(
                                        r#"{{"device_id":"{}","state":"pulsing","duration_us":{},"period_us":{},"brightness_percent":{},"wifi_ssid":"{}","pulse_count":{},"config_age_secs":{:.1},"last_pulse_secs":{:.1}}}"#,
                                        chip_id_for_status,
                                        config.duration_us,
                                        config.period_us,
                                        config.brightness_percent,
                                        wifi_ssid,
                                        stats.pulse_count,
                                        stats.config_changed_time.elapsed().as_secs_f32(),
                                        last_pulse_secs
                                    )
                                }
                                esp32_led_flasher::led::LedStatus::SlowBlink => {
                                    format!(
                                        r#"{{"device_id":"{}","state":"blink","frequency_hz":1,"wifi_ssid":"{}","pulse_count":{},"config_age_secs":{:.1}}}"#,
                                        chip_id_for_status,
                                        wifi_ssid,
                                        stats.pulse_count,
                                        stats.config_changed_time.elapsed().as_secs_f32()
                                    )
                                }
                                esp32_led_flasher::led::LedStatus::FastBlink => {
                                    format!(
                                        r#"{{"device_id":"{}","state":"blink","frequency_hz":5,"wifi_ssid":"{}","pulse_count":{},"config_age_secs":{:.1}}}"#,
                                        chip_id_for_status,
                                        wifi_ssid,
                                        stats.pulse_count,
                                        stats.config_changed_time.elapsed().as_secs_f32()
                                    )
                                }
                            };

                            // Publish status response
                            if let Err(e) = mqtt_for_status.publish(
                                &status_topic,
                                status_json.as_bytes(),
                                QoS::AtMostOnce,
                                false,
                            ) {
                                log::warn!("⚠️  Failed to publish status response: {:?}", e);
                            } else {
                                log::info!("📤 Published status response");
                            }
                        }

                        log::info!("📊 MQTT status responder thread exiting");
                    })
                    .expect("Failed to spawn MQTT status responder");

                Some(mqtt_client_arc)
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
    */

    // Initialize UART0 for CLI (USB-C on most ESP32 boards)
    log::info!("📟 Initializing UART0 for CLI...");
    let uart_config = UartConfig::default().baudrate(esp_idf_hal::units::Hertz(115200));
    let mut uart = UartDriver::new(
        peripherals.uart0,
        peripherals.pins.gpio1,  // TX
        peripherals.pins.gpio3,  // RX
        Option::<esp_idf_hal::gpio::Gpio0>::None,
        Option::<esp_idf_hal::gpio::Gpio0>::None,
        &uart_config,
    )?;

    log::info!("✅ UART0 initialized at 115200 baud");

    // Split UART into TX and RX drivers
    let (uart_tx, uart_rx) = uart.split();

    // Create terminal and command handler
    let mut terminal = Terminal::new(uart_tx, uart_rx);
    let mut command_handler = CommandHandler::new().with_led(led_manager.clone());

    if let Some(wifi_ref) = &wifi {
        command_handler = command_handler.with_wifi(wifi_ref.clone());
    }
    if let Some(mqtt_ref) = &mqtt {
        // Set up event topics for MQTT notifications
        let event_config_topic = format!("istorrs/led/{}/event/config", chip_id);
        let event_error_topic = format!("istorrs/led/{}/event/error", chip_id);

        log::info!("📡 MQTT Event Topics:");
        log::info!("   Config events: {}", event_config_topic);
        log::info!("   Error events:  {}", event_error_topic);

        command_handler = command_handler
            .with_mqtt(mqtt_ref.clone())
            .with_event_topics(event_config_topic, event_error_topic);
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
        terminal.write_line(&format!("  TCP CLI: port {} (nc <ip> {})", tcp_cli::TCP_CLI_PORT, tcp_cli::TCP_CLI_PORT))?;
    } else {
        terminal.write_line("  WiFi: ❌ Not connected")?;
        terminal.write_line("  TCP CLI: ❌ Unavailable (no WiFi)")?;
    }

    if mqtt.is_some() {
        terminal.write_line("  MQTT: ✅ Connected")?;
        terminal.write_line(&format!("  Control topics:"))?;
        terminal.write_line(&format!("    {}", mqtt_control_topic_device))?;
        terminal.write_line(&format!("    {}", MQTT_CONTROL_TOPIC_SHARED))?;
    } else {
        terminal.write_line("  MQTT: ❌ Disabled (use mqtt_enable to connect)")?;
    }

    terminal.write_line("")?;
    terminal.write_line("Type 'help' for available commands")?;
    terminal.write_line("")?;

    // Main CLI loop
    log::info!("🚀 Entering main CLI loop");

    // MQTT is now only used for provisioning (receiving config changes)
    // Status is reported only on STATUS_REQ messages

    loop {
        // Read and handle characters from UART
        if let Ok(Some(ch)) = terminal.read_char() {
            if let Ok(Some(command_str)) = terminal.handle_char(ch) {
                // Parse and execute the command
                let command = CommandParser::parse_command(&command_str);
                match command_handler.execute_command(command) {
                    Ok(response) => {
                        if !response.is_empty() {
                            let _ = terminal.write_line(&response);
                        }
                        let _ = terminal.print_prompt();
                    }
                    Err(e) => {
                        let _ = terminal.write_line(&format!("Error: {}", e));
                        let _ = terminal.print_prompt();
                    }
                }
            }
        }

        // Small delay to prevent busy loop
        FreeRtos::delay_ms(10);
    }
}
