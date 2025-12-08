use anyhow::Result;
use esp_idf_svc::mqtt::client::{EspMqttClient, EventPayload, MqttClientConfiguration, QoS};
use log::{info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub type MessageCallback = Arc<dyn Fn(&str, &[u8]) + Send + Sync>;

#[derive(Clone)]
pub struct MqttStatus {
    pub broker_url: String,
    pub client_id: String,
    pub connected: Arc<AtomicBool>,
    pub shutdown: Arc<AtomicBool>, // Signal to stop connection handler thread
    pub last_published_topic: Arc<Mutex<String>>,
    pub last_received_topic: Arc<Mutex<String>>,
    pub last_received_message: Arc<Mutex<String>>,
    pub subscriptions: Arc<Mutex<Vec<String>>>,
    pub publish_count: Arc<Mutex<u32>>,
    pub receive_count: Arc<Mutex<u32>>,
}

impl Default for MqttStatus {
    fn default() -> Self {
        Self {
            broker_url: String::new(),
            client_id: String::new(),
            connected: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            last_published_topic: Arc::new(Mutex::new(String::new())),
            last_received_topic: Arc::new(Mutex::new(String::new())),
            last_received_message: Arc::new(Mutex::new(String::new())),
            subscriptions: Arc::new(Mutex::new(Vec::new())),
            publish_count: Arc::new(Mutex::new(0)),
            receive_count: Arc::new(Mutex::new(0)),
        }
    }
}

pub struct MqttClient {
    client: Arc<Mutex<EspMqttClient<'static>>>,
    status: MqttStatus,
}

impl MqttClient {
    pub fn new(
        broker_url: &str,
        client_id: &str,
        username: Option<&str>,
        password: Option<&str>,
        message_callback: MessageCallback,
    ) -> Result<Self> {
        info!("Initializing MQTT client...");
        info!("  Broker: {}", broker_url);
        info!("  Client ID: {}", client_id);
        if username.is_some() {
            info!("  Using authentication");
        }

        let status = MqttStatus {
            broker_url: broker_url.to_string(),
            client_id: client_id.to_string(),
            ..Default::default()
        };

        let mqtt_config = MqttClientConfiguration {
            client_id: Some(client_id),
            username: username,
            password: password,
            keep_alive_interval: Some(std::time::Duration::from_secs(30)),
            // Set to 5 minutes to avoid spamming errors when broker is down
            reconnect_timeout: Some(std::time::Duration::from_secs(300)),
            ..Default::default()
        };

        let (client, mut connection) = EspMqttClient::new(broker_url, &mqtt_config)?;

        info!("MQTT client created, spawning connection handler");

        let status_clone = status.clone();

        // Spawn connection handler thread
        std::thread::Builder::new()
            .stack_size(8192)
            .name("mqtt_conn".to_string())
            .spawn(move || {
                info!("MQTT connection handler started");
                let mut consecutive_errors = 0u32;
                let mut last_error_log_time = std::time::Instant::now();

                loop {
                    // Check if we've been signaled to shut down
                    if status_clone.shutdown.load(Ordering::Relaxed) {
                        info!(
                            "🔌 MQTT connection handler received shutdown signal, exiting cleanly"
                        );
                        break;
                    }

                    match connection.next() {
                        Ok(event) => match event.payload() {
                            EventPayload::Connected(session_present) => {
                                info!(
                                    "✅ MQTT connected to broker (session_present: {})",
                                    session_present
                                );
                                status_clone.connected.store(true, Ordering::Relaxed);
                                consecutive_errors = 0; // Reset error counter on success
                            }
                            EventPayload::Disconnected => {
                                info!("🔌 MQTT disconnected from broker");
                                status_clone.connected.store(false, Ordering::Relaxed);
                                // In on-demand mode, disconnect is intentional - exit thread
                                info!("🔌 MQTT connection handler exiting (clean disconnect)");
                                break;
                            }
                            EventPayload::Received {
                                topic: Some(topic_str),
                                data,
                                ..
                            } => {
                                if let Ok(msg_str) = std::str::from_utf8(data) {
                                    info!("📩 MQTT received on '{}': {}", topic_str, msg_str);
                                    *status_clone.last_received_topic.lock().unwrap() =
                                        topic_str.to_string();
                                    *status_clone.last_received_message.lock().unwrap() =
                                        msg_str.to_string();
                                    *status_clone.receive_count.lock().unwrap() += 1;
                                } else {
                                    info!(
                                        "📩 MQTT received on '{}': {} bytes (non-UTF8)",
                                        topic_str,
                                        data.len()
                                    );
                                }
                                message_callback(topic_str, data);
                            }
                            EventPayload::Received { topic: None, .. } => {
                                // Reduce log spam for this common case
                            }
                            EventPayload::Subscribed(id) => {
                                info!("✅ MQTT subscribed (message id: {})", id);
                            }
                            EventPayload::Published(id) => {
                                info!("✅ MQTT published (message id: {})", id);
                            }
                            EventPayload::Error(e) => {
                                // Rate limit error logging to reduce spam
                                if last_error_log_time.elapsed().as_secs() >= 10 {
                                    warn!("❌ MQTT error: {:?}", e);
                                    last_error_log_time = std::time::Instant::now();
                                }
                            }
                            EventPayload::BeforeConnect => {
                                // Rate limit BeforeConnect logging
                                if consecutive_errors == 0
                                    || last_error_log_time.elapsed().as_secs() >= 30
                                {
                                    info!("🔄 MQTT attempting to connect...");
                                    last_error_log_time = std::time::Instant::now();
                                }
                            }
                            _ => {
                                // Reduce log spam for other events
                            }
                        },
                        Err(e) => {
                            status_clone.connected.store(false, Ordering::Relaxed);
                            consecutive_errors += 1;

                            // Check if this is an INVALID_STATE error (client intentionally disconnected)
                            // If so, exit the thread gracefully after a few attempts
                            let error_str = format!("{:?}", e);
                            let is_invalid_state = error_str.contains("INVALID_STATE");

                            if is_invalid_state && consecutive_errors >= 3 {
                                // Client was intentionally disconnected (on-demand mode)
                                // Exit thread gracefully instead of continuing to retry
                                info!("🔌 MQTT connection handler exiting (client disconnected)");
                                break;
                            }

                            // Exponential backoff: 1s, 2s, 5s, 10s, 30s, 60s, then 300s (5 min) max
                            let backoff_secs = match consecutive_errors {
                                1 => 1,
                                2 => 2,
                                3 => 5,
                                4 => 10,
                                5 => 30,
                                6 => 60,
                                _ => 300, // 5 minutes to avoid spamming serial port
                            };

                            // Don't log INVALID_STATE errors (expected in on-demand mode)
                            // Rate limit other errors
                            if !is_invalid_state
                                && (consecutive_errors <= 3
                                    || last_error_log_time.elapsed().as_secs() >= 30)
                            {
                                warn!(
                                    "❌ MQTT connection error (#{}, retry in {}s): {:?}",
                                    consecutive_errors, backoff_secs, e
                                );
                                last_error_log_time = std::time::Instant::now();
                            }

                            std::thread::sleep(std::time::Duration::from_secs(backoff_secs));
                        }
                    }
                }
            })?;

        // Transmute to 'static - the client will live for the entire program
        let client_static: EspMqttClient<'static> = unsafe { std::mem::transmute(client) };

        Ok(Self {
            client: Arc::new(Mutex::new(client_static)),
            status,
        })
    }

    pub fn get_status(&self) -> MqttStatus {
        self.status.clone()
    }

    pub fn is_connected(&self) -> bool {
        self.status.connected.load(Ordering::Relaxed)
    }

    pub fn publish(&self, topic: &str, data: &[u8], qos: QoS, retain: bool) -> Result<()> {
        self.client
            .lock()
            .unwrap()
            .enqueue(topic, qos, retain, data)?;

        *self.status.last_published_topic.lock().unwrap() = topic.to_string();
        *self.status.publish_count.lock().unwrap() += 1;

        info!(
            "📤 MQTT enqueued publish to '{}': {} bytes",
            topic,
            data.len()
        );
        Ok(())
    }

    pub fn subscribe(&self, topic: &str, qos: QoS) -> Result<()> {
        self.client.lock().unwrap().subscribe(topic, qos)?;

        let mut subs = self.status.subscriptions.lock().unwrap();
        if !subs.contains(&topic.to_string()) {
            subs.push(topic.to_string());
        }

        info!("📥 MQTT subscribe requested for topic: '{}'", topic);
        Ok(())
    }

    pub fn unsubscribe(&self, topic: &str) -> Result<()> {
        self.client.lock().unwrap().unsubscribe(topic)?;

        let mut subs = self.status.subscriptions.lock().unwrap();
        subs.retain(|s| s != topic);

        info!("MQTT unsubscribed from topic: '{}'", topic);
        Ok(())
    }

    pub fn shutdown(&self) {
        info!("🔌 MQTT: Signaling connection handler to shutdown...");
        self.status.shutdown.store(true, Ordering::Relaxed);
        self.status.connected.store(false, Ordering::Relaxed);

        // Give the thread a moment to see the shutdown signal and exit
        std::thread::sleep(std::time::Duration::from_millis(100));
        info!("✅ MQTT: Shutdown signal sent");
    }
}
