use esp32_water_meter::cli::{CommandHandler, CommandParser, Terminal};
use esp32_water_meter::mqtt::MqttClient;
use esp32_water_meter::mtu::{GpioMtuTimerV2, MtuCommand, MtuConfig};
use esp32_water_meter::wifi::WifiManager;
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{Input, Output, PinDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::uart::{config::Config as UartConfig, UartDriver};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::mqtt::client::QoS;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sntp;
use esp_idf_svc::sys;
use std::sync::{Arc, Condvar, Mutex};
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

    log::info!("ESP32 Water Meter MTU Interface with CLI");
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
    const MQTT_PUBLISH_TOPIC: &str = "istorrs/mtu/data";
    const MQTT_CONTROL_TOPIC_SHARED: &str = "istorrs/mtu/control"; // Shared topic for broadcast commands

    // Device-specific MQTT topics based on chip ID
    let mqtt_client_id = format!("esp32-mtu-{}", chip_id.replace(":", ""));
    let mqtt_control_topic_device = format!("istorrs/mtu/{}/control", chip_id);

    log::info!("📡 MQTT Client ID: {}", mqtt_client_id);
    log::info!("📡 MQTT Control Topics:");
    log::info!("   Shared:  {}", MQTT_CONTROL_TOPIC_SHARED);
    log::info!("   Device:  {}", mqtt_control_topic_device);

    // Initialize WiFi manager but don't connect yet (on-demand connection)
    let wifi = if WIFI_SSID != "YOUR_SSID" {
        log::info!("🌐 Initializing WiFi manager (on-demand mode)...");
        log::info!("  SSID: {}", WIFI_SSID);
        log::info!("  Password length: {} chars", WIFI_PASSWORD.len());

        match WifiManager::new(
            peripherals.modem,
            sysloop.clone(),
            nvs.clone(),
            WIFI_SSID,
            WIFI_PASSWORD,
        ) {
            Ok(mut wifi) => {
                log::info!("✅ WiFi manager created");

                // Disconnect immediately for on-demand usage
                log::info!("🔌 Disconnecting WiFi (will reconnect on-demand for MQTT publish)");
                if let Err(e) = wifi.disconnect() {
                    log::warn!("⚠️  WiFi disconnect failed: {:?}", e);
                }

                Some(Arc::new(Mutex::new(wifi)))
            }
            Err(e) => {
                log::error!("❌ WiFi initialization failed: {:?}", e);
                log::warn!("⚠️  Continuing without WiFi - use 'wifi_connect' command to retry");
                log::warn!(
                    "⚠️  Note: WiFi requires modem peripheral which is consumed on first init"
                );
                log::warn!("⚠️  Recommendation: Fix WiFi credentials and reboot");
                None
            }
        }
    } else {
        log::info!("WiFi disabled (update WIFI_SSID/WIFI_PASSWORD to enable)");
        None
    };

    // Initialize UART0 for CLI (USB-C connection)
    log::info!("Initializing UART0 for CLI (USB-C)...");
    let uart_config = UartConfig::new().baudrate(115200.into());
    let mut uart = UartDriver::new(
        peripherals.uart0,
        peripherals.pins.gpio1, // TX (U0TXD)
        peripherals.pins.gpio3, // RX (U0RXD)
        Option::<esp_idf_hal::gpio::Gpio0>::None,
        Option::<esp_idf_hal::gpio::Gpio0>::None,
        &uart_config,
    )?;

    // Split UART into tx and rx drivers
    let (uart_tx, uart_rx) = uart.split();

    log::info!("✅ UART0 initialized (115200 baud)");

    // Initialize GPIO pins for MTU
    // Using GPIO4 for clock output and GPIO5 for data input
    log::info!("Initializing MTU GPIO pins...");
    log::info!("  Clock pin: GPIO4 (output, starting LOW - no power to meter)");
    log::info!("  Data pin:  GPIO5 (input)");

    // Initialize clock pin LOW to simulate no power to meter at startup
    let mut clock_pin = PinDriver::output(peripherals.pins.gpio4)?;
    clock_pin.set_low()?;
    log::info!("✅ Clock pin initialized LOW");

    let data_pin = PinDriver::input(peripherals.pins.gpio5)?;

    // SAFETY: We need 'static lifetime for pins to move into background thread
    // The pins will be owned by the MTU thread for the entire program lifetime
    let clock_pin_static: PinDriver<'static, esp_idf_hal::gpio::Gpio4, Output> =
        unsafe { core::mem::transmute(clock_pin) };
    let data_pin_static: PinDriver<'static, esp_idf_hal::gpio::Gpio5, Input> =
        unsafe { core::mem::transmute(data_pin) };

    // Get timer peripheral for MTU
    let timer = peripherals.timer00;

    // Create MTU instance with default config
    let config = MtuConfig::default();
    let mtu = Arc::new(GpioMtuTimerV2::new(config));

    log::info!("✅ MTU GPIO pins configured");
    log::info!("✅ MTU instance created with {} baud", mtu.get_baud_rate());

    // Spawn MTU background thread and get command sender
    let mtu_cmd_sender = GpioMtuTimerV2::spawn_mtu_thread(
        Arc::clone(&mtu),
        clock_pin_static,
        data_pin_static,
        timer,
    );

    log::info!("✅ MTU background thread spawned");

    // MQTT will be created on-demand when publishing data
    log::info!("📡 MQTT: On-demand mode (will connect only when publishing)");

    // Initialize CLI components
    let mut terminal = Terminal::new(uart_tx, uart_rx);
    let mut command_handler =
        CommandHandler::new().with_mtu(Arc::clone(&mtu), mtu_cmd_sender.clone());

    // Add WiFi to command handler if available
    if let Some(ref wifi_manager) = wifi {
        command_handler = command_handler.with_wifi(Arc::clone(wifi_manager));
    }

    log::info!("✅ CLI initialized");

    // Send welcome message
    terminal.write_line("")?;
    terminal.write_line("ESP32 Water Meter MTU Interface")?;
    terminal.write_line("Type 'help' for available commands")?;
    terminal.write_line("Use TAB for command autocompletion")?;
    terminal.write_line("MTU Clock: GPIO4 | Data: GPIO5")?;

    // Show WiFi/MQTT status in welcome message
    if wifi.is_some() {
        terminal.write_line("WiFi: On-demand (disconnected)")?;
        terminal.write_line("MQTT: On-demand (will connect after MTU read)")?;
    }
    terminal.print_prompt()?;

    log::info!("Entering CLI loop...");

    // Helper function to publish MTU data with on-demand WiFi/MQTT connection
    // This function connects WiFi, creates MQTT client, publishes data,
    // waits for downlink messages, then disconnects everything
    let publish_with_connectivity = |wifi_manager: &Arc<Mutex<WifiManager>>,
                                     mtu_sender: &std::sync::mpsc::Sender<MtuCommand>,
                                     message: &str,
                                     stats: (u32, u32, usize),
                                     baud_rate: u32,
                                     counter: &mut u32,
                                     control_shared: &str,
                                     control_device: &str,
                                     client_id: &str| {
        let (successful, corrupted, cycles) = stats;

        log::info!("📡 On-demand publish: Connecting WiFi...");

        // Step 1: Connect WiFi
        let wifi_result = if let Ok(mut wifi_guard) = wifi_manager.lock() {
            wifi_guard.reconnect(None, None)
        } else {
            log::error!("❌ Failed to lock WiFi manager");
            return;
        };

        if let Err(e) = wifi_result {
            log::error!("❌ WiFi connection failed: {:?}", e);
            return;
        }

        log::info!("✅ WiFi connected");

        // Step 2: Synchronize time with NTP server
        log::info!("🕒 Synchronizing time with NTP server...");
        let sync = Arc::new((Mutex::new(false), Condvar::new()));
        let sntp = {
            let sync = sync.clone();
            sntp::EspSntp::new_with_callback(&sntp::SntpConf::default(), move |_| {
                let (lock, cvar) = &*sync;
                let mut guard = lock.lock().unwrap();
                *guard = true;
                cvar.notify_one();
            })
            .unwrap()
        };

        // Wait for sync with 10-second timeout
        let (lock, cvar) = &*sync;
        let mut guard = lock.lock().unwrap();
        let mut synced = *guard;

        if !synced {
            let (new_guard, timeout_result) =
                cvar.wait_timeout(guard, Duration::from_secs(10)).unwrap();
            guard = new_guard;
            synced = *guard && !timeout_result.timed_out();
        }

        if synced {
            log::info!("✅ Time synchronized with NTP");
        } else {
            log::warn!("⚠️  Time synchronization timed out (continuing anyway)");
        }
        drop(guard);
        drop(sntp);

        // Step 3: Create MQTT client with message handler for control topic
        log::info!("📡 Creating MQTT client...");

        // Clone MTU sender for MQTT callback
        let mqtt_mtu_sender = mtu_sender.clone();

        // Clone control topics for callback
        let callback_control_shared = control_shared.to_string();
        let callback_control_device = control_device.to_string();

        let mqtt_client = match MqttClient::new(
            MQTT_BROKER,
            client_id,
            Arc::new(move |topic, data| {
                if let Ok(msg) = std::str::from_utf8(data) {
                    log::info!("📩 MQTT control message on {}: {}", topic, msg);

                    // Accept commands from both shared and device-specific topics
                    if topic == callback_control_shared || topic == callback_control_device {
                        // Try to parse as JSON first
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(msg) {
                            // Handle JSON messages like {"baud_rate": 1200}
                            if let Some(baud_rate) = json.get("baud_rate").and_then(|v| v.as_u64())
                            {
                                log::info!("MQTT: Setting baud rate to {} bps", baud_rate);
                                let _ = mqtt_mtu_sender.send(MtuCommand::SetBaudRate {
                                    baud_rate: baud_rate as u32,
                                });
                            }
                            if let Some(cmd) = json.get("command").and_then(|v| v.as_str()) {
                                match cmd {
                                    "start" => {
                                        let duration = json
                                            .get("duration")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(30);
                                        log::info!("MQTT: Starting MTU for {}s", duration);
                                        let _ = mqtt_mtu_sender.send(MtuCommand::Start {
                                            duration_secs: duration,
                                        });
                                    }
                                    "stop" => {
                                        log::info!("MQTT: Stopping MTU");
                                        let _ = mqtt_mtu_sender.send(MtuCommand::Stop);
                                    }
                                    _ => {
                                        log::warn!("MQTT: Unknown JSON command: {}", cmd);
                                    }
                                }
                            }
                        } else {
                            // Fall back to plain text commands for backwards compatibility
                            let cmd = msg.trim().to_lowercase();
                            match cmd.as_str() {
                                "start" => {
                                    log::info!("MQTT: Starting MTU (30s default)");
                                    let _ = mqtt_mtu_sender
                                        .send(MtuCommand::Start { duration_secs: 30 });
                                }
                                msg if msg.starts_with("start ") => {
                                    if let Some(duration_str) = msg.strip_prefix("start ") {
                                        if let Ok(duration) = duration_str.parse::<u64>() {
                                            log::info!("MQTT: Starting MTU for {}s", duration);
                                            let _ = mqtt_mtu_sender.send(MtuCommand::Start {
                                                duration_secs: duration,
                                            });
                                        }
                                    }
                                }
                                "stop" => {
                                    log::info!("MQTT: Stopping MTU");
                                    let _ = mqtt_mtu_sender.send(MtuCommand::Stop);
                                }
                                _ => {
                                    log::warn!("MQTT: Unknown control command: {}", cmd);
                                }
                            }
                        }
                    }
                }
            }),
        ) {
            Ok(client) => client,
            Err(e) => {
                log::error!("❌ MQTT client creation failed: {:?}", e);
                // Disconnect WiFi before returning
                if let Ok(mut wifi_guard) = wifi_manager.lock() {
                    let _ = wifi_guard.disconnect();
                }
                return;
            }
        };

        // Step 4: Wait for MQTT connection (up to 10 seconds)
        log::info!("⏳ Waiting for MQTT connection...");
        for i in 0..20 {
            if mqtt_client.is_connected() {
                log::info!("✅ MQTT connected");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            if i == 19 {
                log::error!("❌ MQTT connection timeout");
                // Disconnect WiFi and return
                if let Ok(mut wifi_guard) = wifi_manager.lock() {
                    let _ = wifi_guard.disconnect();
                }
                return;
            }
        }

        // Step 5: Subscribe to control topics (both shared and device-specific)
        log::info!("📥 Subscribing to shared control topic: {}", control_shared);
        if let Err(e) = mqtt_client.subscribe(control_shared, QoS::AtLeastOnce) {
            log::warn!("⚠️  Failed to subscribe to shared control topic: {:?}", e);
        }

        log::info!("📥 Subscribing to device control topic: {}", control_device);
        if let Err(e) = mqtt_client.subscribe(control_device, QoS::AtLeastOnce) {
            log::warn!("⚠️  Failed to subscribe to device control topic: {:?}", e);
        }

        // Step 6: Get timestamp
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let timestamp_str =
            chrono::DateTime::from_timestamp(now.as_secs() as i64, now.subsec_nanos())
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| "<unknown>".to_string());

        // Step 7: Publish MTU data with device identification and timestamp
        let chip_id = get_chip_id();
        let (wifi_mac, wifi_ip) = if let Ok(wifi_guard) = wifi_manager.lock() {
            let mac = wifi_guard
                .get_mac()
                .unwrap_or_else(|_| "unknown".to_string());
            let ip = wifi_guard
                .get_ip()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            (mac, ip)
        } else {
            ("unknown".to_string(), "unknown".to_string())
        };

        let payload = serde_json::json!({
            "timestamp": timestamp_str,
            "chip_id": chip_id,
            "wifi_mac": wifi_mac,
            "wifi_ip": wifi_ip,
            "message": message,
            "baud_rate": baud_rate,
            "cycles": cycles,
            "successful": successful,
            "corrupted": corrupted,
            "count": *counter,
        });

        if let Ok(json_str) = serde_json::to_string(&payload) {
            match mqtt_client.publish(
                MQTT_PUBLISH_TOPIC,
                json_str.as_bytes(),
                QoS::AtLeastOnce,
                false,
            ) {
                Ok(_) => {
                    *counter += 1;
                    log::info!(
                        "📤 Published #{} to {}: {}",
                        *counter,
                        MQTT_PUBLISH_TOPIC,
                        message
                    );
                }
                Err(e) => {
                    log::error!("❌ MQTT publish failed: {:?}", e);
                }
            }
        }

        // Step 8: Wait 5 seconds for queued downlink messages
        log::info!("⏳ Waiting 5s for queued downlink messages...");
        std::thread::sleep(std::time::Duration::from_secs(5));

        // Step 9: Signal MQTT connection handler to shutdown (prevents errors/retries)
        mqtt_client.shutdown();

        // Drop the client (connection handler already exited cleanly)
        drop(mqtt_client);

        // Step 10: Disconnect WiFi
        log::info!("🔌 Disconnecting WiFi...");
        if let Ok(mut wifi_guard) = wifi_manager.lock() {
            if let Err(e) = wifi_guard.disconnect() {
                log::warn!("⚠️  WiFi disconnect failed: {:?}", e);
            }
        }

        log::info!("✅ On-demand publish cycle complete");
    };

    // Track last published cycle count for on-demand publishing
    // Publish based on MTU read cycles, not message content (allows duplicate messages)
    let mut last_published_cycles = 0u64;
    let mut publish_counter = 0u32;

    // Main CLI loop
    loop {
        // On-demand publish: Connect WiFi/MQTT only when new MTU data is available
        if let Some(wifi_manager) = &wifi {
            if let Some(current_message) = mtu.get_last_message() {
                // Get statistics for the JSON payload
                let (successful, corrupted, cycles) = mtu.get_stats();

                // Publish if we have a new MTU read cycle (successful or corrupted count increased)
                let total_reads = successful + corrupted;
                let should_publish = u64::from(total_reads) > last_published_cycles;

                if should_publish {
                    let baud_rate = mtu.get_baud_rate();

                    // Call on-demand publish function
                    // This will: connect WiFi → create MQTT → publish → wait for downlink → disconnect
                    publish_with_connectivity(
                        wifi_manager,
                        &mtu_cmd_sender,
                        current_message.as_str(),
                        (successful, corrupted, cycles),
                        baud_rate,
                        &mut publish_counter,
                        MQTT_CONTROL_TOPIC_SHARED,
                        &mqtt_control_topic_device,
                        &mqtt_client_id,
                    );

                    // Update last published cycle count
                    last_published_cycles = u64::from(total_reads);
                }
            }
        }

        // Read character with non-blocking timeout
        match terminal.read_char() {
            Ok(Some(ch)) => {
                // Handle character and check if we got a complete command
                match terminal.handle_char(ch) {
                    Ok(Some(command_line)) => {
                        // Parse and execute the command
                        let command = CommandParser::parse_command(&command_line);

                        // Clone command for later pattern matching
                        let command_clone = command.clone();

                        match command_handler.execute_command(command) {
                            Ok(response) => {
                                if !response.is_empty() {
                                    let _ = terminal.write_line(&response);
                                }
                            }
                            Err(_) => {
                                log::warn!("CLI command execution error");
                                let _ = terminal.write_line("Command execution error.");
                            }
                        }

                        // Handle special commands that need terminal interaction
                        match command_clone {
                            esp32_water_meter::cli::CliCommand::Help => {
                                let _ = terminal.show_help();
                            }
                            esp32_water_meter::cli::CliCommand::Clear => {
                                let _ = terminal.clear_screen();
                            }
                            _ => {}
                        }

                        let _ = terminal.print_prompt();
                    }
                    Ok(None) => {
                        // Character processed but no complete command yet
                    }
                    Err(_) => {
                        log::warn!("Terminal input error");
                        let _ = terminal.write_line("Input error");
                        let _ = terminal.print_prompt();
                    }
                }
            }
            Ok(None) => {
                // No data available, small delay to avoid busy loop
                FreeRtos::delay_ms(10);
            }
            Err(_) => {
                // UART error, small delay
                FreeRtos::delay_ms(10);
            }
        }
    }
}
