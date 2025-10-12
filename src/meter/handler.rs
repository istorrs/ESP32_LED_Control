use super::config::{MeterConfig, MeterType};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use esp_idf_hal::gpio::{Input, Level, Output, Pin, PinDriver};
use esp_idf_hal::task::notification::Notification;
use heapless::String;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};

pub struct MeterHandler {
    config: Mutex<MeterConfig>,
    pulse_count: Arc<AtomicUsize>,
    bits_transmitted: Arc<AtomicUsize>,
    messages_sent: Arc<AtomicUsize>,
    transmitting: Arc<AtomicBool>,
}

impl MeterHandler {
    pub fn new(config: MeterConfig) -> Self {
        Self {
            config: Mutex::new(config),
            pulse_count: Arc::new(AtomicUsize::new(0)),
            bits_transmitted: Arc::new(AtomicUsize::new(0)),
            messages_sent: Arc::new(AtomicUsize::new(0)),
            transmitting: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn get_config(&self) -> MeterConfig {
        let config = self.config.lock().unwrap();
        config.clone()
    }

    pub fn set_type(&self, meter_type: MeterType) {
        let mut config = self.config.lock().unwrap();
        config.meter_type = meter_type;
        log::info!("Meter: Type set to {:?}", config.meter_type);
    }

    pub fn set_message(&self, message: String<256>) {
        let mut config = self.config.lock().unwrap();
        config.response_message = message;
        log::info!("Meter: Response message updated");
    }

    pub fn get_uart_format(&self) -> crate::uart_format::UartFormat {
        let config = self.config.lock().unwrap();
        config.uart_format
    }

    pub fn set_uart_format(&self, format: crate::uart_format::UartFormat) {
        let mut config = self.config.lock().unwrap();
        config.uart_format = format;
        log::info!("Meter: UART format set to {}", format.as_str());
    }

    pub fn enable(&self) {
        let mut config = self.config.lock().unwrap();
        config.enabled = true;
        log::info!("Meter: Enabled");
    }

    pub fn disable(&self) {
        let mut config = self.config.lock().unwrap();
        config.enabled = false;
        log::info!("Meter: Disabled");
    }

    pub fn is_enabled(&self) -> bool {
        let config = self.config.lock().unwrap();
        config.enabled
    }

    /// Build UART frame with proper framing using configured UART format
    fn build_uart_frame(&self, byte: u8, uart_format: crate::uart_format::UartFormat) -> heapless::Vec<u8, 12> {
        // Use the uart_format module's encode function
        let bits = crate::uart_format::encode_uart_frame(byte, uart_format);

        // Convert std::vec::Vec to heapless::Vec
        let mut heapless_bits: heapless::Vec<u8, 12> = heapless::Vec::new();
        for &bit in &bits {
            let _ = heapless_bits.push(bit);
        }
        heapless_bits
    }

    /// Build complete response frame buffer for all characters in the message
    pub fn build_response_frames(&self) -> heapless::Vec<u8, 2048> {
        let config = self.config.lock().unwrap();
        let mut frame_buffer = heapless::Vec::new();

        // Build frames for each character in the response message
        for (char_index, ch) in config.response_message.chars().enumerate() {
            let char_frame = self.build_uart_frame(ch as u8, config.uart_format);
            log::info!(
                "Meter: Building frame for char #{}: '{}' (ASCII {}) -> {} bits [{}]",
                char_index + 1,
                ch,
                ch as u8,
                char_frame.len(),
                config.uart_format.as_str()
            );
            for &bit in &char_frame {
                let _ = frame_buffer.push(bit);
            }
        }

        log::info!(
            "Meter: Complete frame buffer: {} total bits for {} characters (format: {})",
            frame_buffer.len(),
            config.response_message.len(),
            config.uart_format.as_str()
        );
        frame_buffer
    }

    /// Get meter statistics
    pub fn get_stats(&self) -> (usize, usize, usize, bool) {
        (
            self.pulse_count.load(Ordering::Relaxed),
            self.bits_transmitted.load(Ordering::Relaxed),
            self.messages_sent.load(Ordering::Relaxed),
            self.transmitting.load(Ordering::Relaxed),
        )
    }

    /// Reset meter statistics
    pub fn reset_stats(&self) {
        self.pulse_count.store(0, Ordering::Relaxed);
        self.bits_transmitted.store(0, Ordering::Relaxed);
        self.messages_sent.store(0, Ordering::Relaxed);
        log::info!("Meter: Statistics reset");
    }

    /// Spawn meter background thread that responds to clock signals
    /// Returns nothing - thread runs continuously
    pub fn spawn_meter_thread<P1, P2>(
        meter: Arc<Self>,
        mut clock_pin: PinDriver<'static, P1, Input>,
        mut data_pin: PinDriver<'static, P2, Output>,
    ) where
        P1: Pin,
        P2: Pin,
    {
        std::thread::Builder::new()
            .stack_size(16384) // 16KB stack
            .name("meter_thread".to_string())
            .spawn(move || {
                log::info!("Meter: Background thread started");

                // Create notification for clock interrupt
                let notification = Notification::new();
                let notifier = notification.notifier();

                // Subscribe to clock pin rising edge interrupts
                // Safety: Only accesses notification which is Send+Sync
                unsafe {
                    clock_pin
                        .subscribe(move || {
                            // Minimal ISR work - just notify task
                            notifier.notify_and_yield(NonZeroU32::new(1).unwrap());
                        })
                        .expect("Failed to subscribe to clock pin interrupt");
                }

                log::info!("Meter: Clock pin interrupt configured");

                // Main meter loop
                const WAKE_UP_THRESHOLD: usize = 10; // Pulses to start transmission
                let mut bit_index = 0usize;
                let mut response_bits: heapless::Vec<u8, 2048> = heapless::Vec::new();

                // Set data pin HIGH for idle
                data_pin.set_high().ok();
                log::info!("Meter: Ready - waiting for clock signals");

                loop {
                    // Wait for clock pulse notification from ISR
                    notification.wait(u32::MAX);

                    // Check if meter is enabled
                    if !meter.is_enabled() {
                        continue;
                    }

                    // Increment pulse count
                    let pulse_count = meter.pulse_count.fetch_add(1, Ordering::Relaxed) + 1;

                    // Check if we should start transmitting
                    if !meter.transmitting.load(Ordering::Relaxed) {
                        if pulse_count >= WAKE_UP_THRESHOLD {
                            // Build response frames if needed
                            if response_bits.is_empty() {
                                log::info!(
                                    "Meter: Wake-up threshold reached, building response frames"
                                );
                                response_bits = meter.build_response_frames();
                            }

                            if !response_bits.is_empty() {
                                meter.transmitting.store(true, Ordering::Relaxed);

                                // Set first bit immediately
                                let bit = response_bits[0];
                                data_pin
                                    .set_level(if bit == 1 { Level::High } else { Level::Low })
                                    .ok();
                                meter.bits_transmitted.fetch_add(1, Ordering::Relaxed);
                                bit_index = 1;

                                log::info!(
                                    "Meter: Started transmission - {} total bits to send",
                                    response_bits.len()
                                );
                            }
                        }
                        continue;
                    }

                    // If transmitting, send next bit
                    if bit_index < response_bits.len() {
                        let bit = response_bits[bit_index];
                        data_pin
                            .set_level(if bit == 1 { Level::High } else { Level::Low })
                            .ok();
                        meter.bits_transmitted.fetch_add(1, Ordering::Relaxed);
                        bit_index += 1;

                        // Check if transmission complete
                        if bit_index >= response_bits.len() {
                            meter.transmitting.store(false, Ordering::Relaxed);
                            meter.messages_sent.fetch_add(1, Ordering::Relaxed);
                            meter.pulse_count.store(0, Ordering::Relaxed);
                            bit_index = 0;
                            data_pin.set_high().ok(); // Return to idle

                            log::info!(
                                "Meter: Transmission complete - {} bits sent",
                                response_bits.len()
                            );
                            log::info!(
                                "Meter: Total messages sent: {}",
                                meter.messages_sent.load(Ordering::Relaxed)
                            );

                            // Clear response buffer to rebuild on next wake-up
                            response_bits.clear();
                        }
                    }
                }
            })
            .expect("Failed to spawn meter thread");

        log::info!("Meter: Background thread spawned successfully");
    }
}
