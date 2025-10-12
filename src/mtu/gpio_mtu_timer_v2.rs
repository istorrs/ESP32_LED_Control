use super::config::MtuConfig;
use super::error::{MtuError, MtuResult};
use super::uart_framing::{extract_char_from_frame, UartFrame};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use esp_idf_hal::gpio::{Input, Output, Pin, PinDriver};
use esp_idf_hal::task::notification::Notification;
use esp_idf_hal::timer::{config::Config as TimerConfig, TimerDriver, TIMER00};
use heapless::String;
use std::num::NonZeroU32;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Commands that can be sent to the MTU background thread
#[derive(Debug, Clone)]
pub enum MtuCommand {
    /// Start MTU operation for specified duration in seconds
    Start { duration_secs: u64 },
    /// Stop MTU operation immediately
    Stop,
    /// Set MTU baud rate (must be stopped to change)
    SetBaudRate { baud_rate: u32 },
    /// Set UART frame format (must be stopped to change)
    SetUartFormat { format: crate::uart_format::UartFormat },
}

/// MTU implementation using hardware timer ISR -> Task pattern
/// ISR handles precise timing, signals task which handles GPIO
pub struct GpioMtuTimerV2 {
    config: Mutex<MtuConfig>,
    running: Arc<AtomicBool>,
    clock_cycles: Arc<AtomicUsize>,
    last_bit: Arc<AtomicU8>,
    last_message: Mutex<Option<String<256>>>,
    message_complete: Arc<AtomicBool>, // Signals when a complete message is received
}

use core::sync::atomic::AtomicU8;

impl GpioMtuTimerV2 {
    pub fn new(config: MtuConfig) -> Self {
        Self {
            config: Mutex::new(config),
            running: Arc::new(AtomicBool::new(false)),
            clock_cycles: Arc::new(AtomicUsize::new(0)),
            last_bit: Arc::new(AtomicU8::new(0)),
            last_message: Mutex::new(None),
            message_complete: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn get_baud_rate(&self) -> u32 {
        let config = self.config.lock().unwrap();
        config.baud_rate
    }

    pub fn set_baud_rate(&self, baud_rate: u32) {
        let mut config = self.config.lock().unwrap();
        config.baud_rate = baud_rate;
    }

    pub fn get_uart_format(&self) -> crate::uart_format::UartFormat {
        let config = self.config.lock().unwrap();
        config.uart_format
    }

    pub fn set_uart_format(&self, format: crate::uart_format::UartFormat) {
        let mut config = self.config.lock().unwrap();
        config.uart_format = format;
    }

    pub fn get_stats(&self) -> (u32, u32, usize) {
        let config = self.config.lock().unwrap();
        let cycles = self.clock_cycles.load(Ordering::Relaxed);
        (config.successful_reads, config.corrupted_reads, cycles)
    }

    pub fn reset_stats(&self) {
        let mut config = self.config.lock().unwrap();
        config.successful_reads = 0;
        config.corrupted_reads = 0;
        self.clock_cycles.store(0, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn get_last_message(&self) -> Option<String<256>> {
        let last_msg = self.last_message.lock().unwrap();
        last_msg.clone()
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Spawn MTU background thread that owns GPIO pins and timer peripheral
    /// Returns a channel sender for sending commands to the MTU thread
    pub fn spawn_mtu_thread<P1, P2>(
        mtu: Arc<Self>,
        mut clock_pin: PinDriver<'static, P1, Output>,
        mut data_pin: PinDriver<'static, P2, Input>,
        timer_peripheral: TIMER00,
    ) -> Sender<MtuCommand>
    where
        P1: Pin,
        P2: Pin,
    {
        let (cmd_tx, cmd_rx): (Sender<MtuCommand>, Receiver<MtuCommand>) = channel();

        std::thread::Builder::new()
            .stack_size(16384) // 16KB stack for MTU thread
            .name("mtu_thread".to_string())
            .spawn(move || {
                log::info!("MTU: Background thread started");

                // Create timer driver once (reusable for all MTU operations)
                let timer_config = TimerConfig::new().auto_reload(true);
                let mut timer_driver: TimerDriver<'static> = unsafe {
                    core::mem::transmute(
                        TimerDriver::new(timer_peripheral, &timer_config)
                            .expect("Failed to create timer driver"),
                    )
                };
                log::info!("MTU: Timer driver created");

                // Create notification once (persistent across all operations)
                let notification = Notification::new();
                let notifier = notification.notifier();

                // Clone Arc for ISR closure (persistent)
                let cycles = mtu.clock_cycles.clone();

                // Subscribe to timer ISR once with persistent references
                // Safety: Only accesses atomics and notification, both are Send+Sync
                unsafe {
                    timer_driver
                        .subscribe(move || {
                            let cycle = cycles.fetch_add(1, Ordering::Relaxed);
                            // 4 phases per bit: 0=HIGH, 1=WAIT, 2=LOW, 3=SAMPLE
                            let phase = (cycle % 4) as u32;
                            if let Some(bits) = NonZeroU32::new(phase + 1) {
                                notifier.notify_and_yield(bits);
                            }
                        })
                        .expect("Failed to subscribe to timer ISR");
                }
                log::info!("MTU: Timer ISR subscription created (persistent)");

                // MTU thread loop - waits for commands
                loop {
                    match cmd_rx.recv() {
                        Ok(MtuCommand::Start { duration_secs }) => {
                            log::info!("MTU: Received Start command for {} seconds", duration_secs);

                            // Run the MTU operation (timer driver and notification are reusable)
                            match mtu.run_mtu_operation_with_timer(
                                &mut clock_pin,
                                &mut data_pin,
                                &mut timer_driver,
                                &notification,
                                duration_secs,
                            ) {
                                Ok(_) => {
                                    log::info!("MTU: Operation completed successfully");
                                }
                                Err(e) => {
                                    log::error!("MTU: Operation failed: {:?}", e);
                                }
                            }
                        }
                        Ok(MtuCommand::Stop) => {
                            log::info!("MTU: Received Stop command");
                            mtu.stop();

                            // Set clock pin LOW to power off meter
                            if let Err(e) = clock_pin.set_low() {
                                log::error!("MTU: Failed to set clock pin LOW: {:?}", e);
                            } else {
                                log::info!("MTU: Clock pin set LOW (power off)");
                            }
                        }
                        Ok(MtuCommand::SetBaudRate { baud_rate }) => {
                            if mtu.is_running() {
                                log::warn!("MTU: Cannot change baud rate while MTU is running");
                            } else if (1..=115200).contains(&baud_rate) {
                                log::info!("MTU: Setting baud rate to {} bps", baud_rate);
                                mtu.set_baud_rate(baud_rate);
                                log::info!("MTU: Baud rate updated to {} bps", baud_rate);
                            } else {
                                log::warn!(
                                    "MTU: Invalid baud rate {} (must be 1-115200)",
                                    baud_rate
                                );
                            }
                        }
                        Ok(MtuCommand::SetUartFormat { format }) => {
                            if mtu.is_running() {
                                log::warn!("MTU: Cannot change UART format while MTU is running");
                            } else {
                                log::info!("MTU: Setting UART format to {}", format.as_str());
                                mtu.set_uart_format(format);
                                log::info!("MTU: UART format updated to {}", format.as_str());
                            }
                        }
                        Err(_) => {
                            // Channel closed - exit thread
                            log::info!("MTU: Command channel closed, thread exiting");
                            break;
                        }
                    }
                }

                log::info!("MTU: Background thread stopped");
            })
            .expect("Failed to spawn MTU thread");

        log::info!("MTU: Background thread spawned successfully");
        cmd_tx
    }

    /// Run MTU operation: ISR generates timing signals, task handles GPIO
    /// Takes a mutable reference to timer driver and notification so they can be reused for subsequent operations
    pub fn run_mtu_operation_with_timer<'a, P1, P2>(
        &self,
        clock_pin: &mut PinDriver<'a, P1, Output>,
        data_pin: &mut PinDriver<'a, P2, Input>,
        timer: &mut TimerDriver<'static>,
        notification: &Notification,
        duration_secs: u64,
    ) -> MtuResult<()>
    where
        P1: esp_idf_hal::gpio::Pin,
        P2: esp_idf_hal::gpio::Pin,
    {
        let config = self.config.lock().unwrap();
        let baud_rate = config.baud_rate;
        let power_up_delay_ms = config.power_up_delay_ms;
        let uart_config = config.clone();
        drop(config);

        log::info!(
            "MTU: Starting ISR->Task timer operation for {} seconds",
            duration_secs
        );
        log::info!("MTU: Baud rate: {} Hz", baud_rate);

        // Set running flag BEFORE spawning UART task so it doesn't exit immediately
        self.running.store(true, Ordering::Relaxed);
        self.clock_cycles.store(0, Ordering::Relaxed);
        self.message_complete.store(false, Ordering::Relaxed); // Reset message completion flag

        // Create bit queue channel for GPIO task -> UART framing task
        let (bit_sender, bit_receiver): (Sender<u8>, Receiver<u8>) = channel();

        // Spawn UART framing task
        let uart_running = self.running.clone();
        let uart_message_complete = self.message_complete.clone();
        let uart_last_message = Arc::new(Mutex::new(None::<String<256>>));
        let uart_last_message_clone = uart_last_message.clone();
        let uart_frame_errors = Arc::new(Mutex::new(0usize));
        let uart_frame_errors_clone = uart_frame_errors.clone();

        let uart_handle = std::thread::Builder::new()
            .stack_size(8192)
            .spawn(move || {
                Self::uart_framing_task(
                    uart_running,
                    uart_message_complete,
                    uart_config,
                    bit_receiver,
                    uart_last_message_clone,
                    uart_frame_errors_clone,
                );
            })
            .map_err(|_| MtuError::GpioError)?;

        log::info!("MTU: UART framing task spawned");

        // Power up sequence
        clock_pin.set_high().map_err(|_| MtuError::GpioError)?;
        log::info!("MTU: Power-up hold {}ms", power_up_delay_ms);
        esp_idf_hal::delay::FreeRtos::delay_ms(power_up_delay_ms as u32);

        // Calculate timer frequency: 4x baud rate (for 4 phases per bit)
        // Phase 0: Set clock HIGH
        // Phase 1: Wait (middle of HIGH phase)
        // Phase 2: Set clock LOW
        // Phase 3: Sample data (middle of LOW phase, before next HIGH)
        let timer_freq_hz = baud_rate * 4;
        let alarm_ticks = timer.tick_hz() / timer_freq_hz as u64;

        log::info!("MTU: Timer tick rate: {} Hz", timer.tick_hz());
        log::info!(
            "MTU: Alarm every {} ticks ({} Hz)",
            alarm_ticks,
            timer_freq_hz
        );

        // Configure and start timer (ISR already subscribed in thread loop)
        timer
            .set_alarm(alarm_ticks)
            .map_err(|_| MtuError::GpioError)?;
        timer.enable_interrupt().map_err(|_| MtuError::GpioError)?;
        timer.enable_alarm(true).map_err(|_| MtuError::GpioError)?;
        timer.enable(true).map_err(|_| MtuError::GpioError)?;

        log::info!("MTU: Timer started, GPIO task running...");

        // Task: Handle GPIO based on notifications from ISR
        let start = std::time::Instant::now();
        let mut last_log_time = start;
        let mut last_cycles = 0usize;
        let mut handled_count = 0usize;
        let mut sample_count = 0usize;
        let mut ones_count = 0usize;
        let mut zeros_count = 0usize;

        // Run until timeout OR until we receive a complete message (like nRF line 367)
        while start.elapsed().as_secs() < duration_secs
            && !self.message_complete.load(Ordering::Relaxed)
        {
            // Wait for notification from ISR (1 tick timeout ~= 1ms)
            if let Some(bitset) = notification.wait(1) {
                handled_count += 1;
                let phase = bitset.get() - 1;

                match phase {
                    0 => {
                        // Phase 0: Set clock HIGH (rising edge)
                        clock_pin.set_high().map_err(|_| MtuError::GpioError)?;
                    }
                    1 => {
                        // Phase 1: Wait (middle of HIGH phase)
                        // No action needed
                    }
                    2 => {
                        // Phase 2: Set clock LOW (falling edge)
                        clock_pin.set_low().map_err(|_| MtuError::GpioError)?;
                    }
                    3 => {
                        // Phase 3: Sample data (middle of LOW phase, before next HIGH)
                        let data_val = data_pin.is_high();
                        let bit = if data_val { 1 } else { 0 };
                        self.last_bit.store(bit, Ordering::Relaxed);

                        sample_count += 1;
                        if bit == 1 {
                            ones_count += 1;
                        } else {
                            zeros_count += 1;
                        }

                        // Send bit to UART framing task
                        // Returns Err if channel is closed (UART task ended)
                        if bit_sender.send(bit).is_err() {
                            // Channel closed - UART task ended
                        }

                        // Log first 20 samples for debugging
                        if sample_count <= 20 {
                            log::info!("MTU: Sample #{}: bit={}", sample_count, bit);
                        }
                    }
                    _ => {}
                }
            }

            // Log status every second
            if start.elapsed().as_secs() > last_log_time.elapsed().as_secs() {
                let current_cycles = self.clock_cycles.load(Ordering::Relaxed);
                let cycles_per_sec = current_cycles - last_cycles;
                last_cycles = current_cycles;
                last_log_time = std::time::Instant::now();

                let elapsed = start.elapsed().as_secs();

                log::info!(
                    "MTU: {}/{}s - ISR: {} ticks, Task: {} handled, {} ticks/sec, Sampled: {} (1s:{}, 0s:{})",
                    elapsed,
                    duration_secs,
                    current_cycles,
                    handled_count,
                    cycles_per_sec,
                    sample_count,
                    ones_count,
                    zeros_count
                );
            }
        }

        // Determine why we exited the loop
        let message_received = self.message_complete.load(Ordering::Relaxed);
        if message_received {
            log::info!("MTU: Data task completed (message received)");
        } else {
            log::warn!("MTU: Operation timeout reached");
        }

        // Stop timer
        self.running.store(false, Ordering::Relaxed);
        timer.enable(false).map_err(|_| MtuError::GpioError)?;

        // Set clock to LOW (power off meter - simulate no power)
        clock_pin.set_low().map_err(|_| MtuError::GpioError)?;
        log::info!("MTU: Clock pin set LOW (power off)");

        let total_cycles = self.clock_cycles.load(Ordering::Relaxed);

        // Close bit channel to signal UART task to exit
        drop(bit_sender);

        // Give UART task a moment to complete and store the message
        // Don't wait indefinitely - the message is already in the shared Arc<Mutex<>>
        log::info!("MTU: Signaling UART framing task to exit...");
        esp_idf_hal::delay::FreeRtos::delay_ms(50);

        // Get the last message and frame error count from UART task (stored in shared Arc)
        let received_message = uart_last_message.lock().unwrap().clone();
        let frame_errors = *uart_frame_errors.lock().unwrap();

        // Don't join the UART thread - it may be stuck in ESP-IDF logging
        // The thread will exit on its own when it completes
        drop(uart_handle);
        log::info!("MTU: UART thread detached (will exit independently)");

        log::info!("MTU: Timer operation completed");
        log::info!("  ISR generated: {} timer ticks", total_cycles);
        log::info!("  Task handled: {} GPIO updates", handled_count);
        log::info!("  Data sampled: {} times", sample_count);
        log::info!(
            "  Bit distribution: {} ones, {} zeros ({:.1}% high)",
            ones_count,
            zeros_count,
            (ones_count as f32 / sample_count as f32) * 100.0
        );
        log::info!(
            "  Efficiency: {:.1}%",
            (handled_count as f32 / total_cycles as f32) * 100.0
        );

        // Update statistics based on message reception
        let mut config = self.config.lock().unwrap();

        // Message is corrupted if we have frame errors OR no message received
        let is_corrupted = frame_errors > 0 || received_message.is_none();

        if let Some(msg) = received_message {
            log::info!("  Received message: '{}'", msg.as_str());

            // Store in our internal state (even if corrupted - might be partially useful)
            let mut last_msg = self.last_message.lock().unwrap();
            *last_msg = Some(msg);

            if is_corrupted {
                // Had frame errors - count as corrupted even though we got a message
                config.corrupted_reads += 1;
                log::warn!(
                    "MTU: Message received but CORRUPTED ({} frame errors) - Successful: {}, Corrupted: {}, Success rate: {:.1}%",
                    frame_errors,
                    config.successful_reads,
                    config.corrupted_reads,
                    (config.successful_reads as f32
                        / (config.successful_reads + config.corrupted_reads) as f32)
                        * 100.0
                );
            } else {
                // Clean message - count as successful
                config.successful_reads += 1;
                log::info!(
                    "MTU: Statistics updated - Successful: {}, Corrupted: {}, Success rate: {:.1}%",
                    config.successful_reads,
                    config.corrupted_reads,
                    (config.successful_reads as f32
                        / (config.successful_reads + config.corrupted_reads) as f32)
                        * 100.0
                );
            }
        } else {
            log::info!("  No complete message received");

            // Increment corrupted reads counter
            config.corrupted_reads += 1;
            log::warn!(
                "MTU: Statistics updated - Successful: {}, Corrupted: {}, Success rate: {:.1}%",
                config.successful_reads,
                config.corrupted_reads,
                (config.successful_reads as f32
                    / (config.successful_reads + config.corrupted_reads) as f32)
                    * 100.0
            );
        }
        drop(config);

        Ok(())
    }

    /// UART framing task - processes bit stream into characters
    /// Follows ESP32C-rust pattern: wait for start bit, collect frame, validate, extract char
    fn uart_framing_task(
        running: Arc<AtomicBool>,
        message_complete: Arc<AtomicBool>,
        config: MtuConfig,
        bit_receiver: Receiver<u8>,
        last_message: Arc<Mutex<Option<String<256>>>>,
        frame_error_count: Arc<Mutex<usize>>,
    ) {
        log::info!("UART: Framing task started");

        // Wait for idle line (consecutive 1-bits) to synchronize to frame boundaries
        // This prevents catching the meter mid-transmission after power-up
        log::info!("UART: Waiting for idle line to synchronize...");
        let mut idle_count = 0;
        const MIN_IDLE_BITS: usize = 10; // Wait for 10 consecutive 1-bits

        while running.load(Ordering::Relaxed) && idle_count < MIN_IDLE_BITS {
            match bit_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(1) => {
                    idle_count += 1;
                }
                Ok(0) => {
                    // Reset if we see a 0 - not yet in idle state
                    idle_count = 0;
                }
                Ok(_) => {
                    // Unexpected bit value
                    idle_count = 0;
                }
                Err(_) => {
                    // Timeout - continue waiting
                }
            }
        }

        if idle_count >= MIN_IDLE_BITS {
            log::info!(
                "UART: Idle line detected ({} consecutive 1-bits), synchronized!",
                idle_count
            );
        } else {
            log::warn!("UART: Failed to detect idle line, proceeding anyway");
        }

        let mut received_chars = heapless::Vec::<char, 256>::new();
        let mut frames_decoded = 0usize;
        let mut frame_errors = 0usize;

        while running.load(Ordering::Relaxed) && !message_complete.load(Ordering::Relaxed) {
            // Wait for start bit (0) - like ESP32C line 511
            let mut found_start = false;
            while running.load(Ordering::Relaxed) && !message_complete.load(Ordering::Relaxed) {
                match bit_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(0) => {
                        found_start = true;
                        break;
                    }
                    Ok(1) => {
                        // Skip idle high bits
                        continue;
                    }
                    Ok(_) => {
                        // Unexpected bit value - should only be 0 or 1
                        log::warn!("UART: Unexpected bit value received");
                        continue;
                    }
                    Err(_) => {
                        // Timeout - check if still running
                        continue;
                    }
                }
            }

            if !found_start || !running.load(Ordering::Relaxed) {
                break;
            }

            // Collect complete frame - like ESP32C lines 538-565
            let frame_size = config.uart_format.total_bits() as usize;
            let mut frame_bits = heapless::Vec::<u8, 16>::new();
            let _ = frame_bits.push(0); // Start bit

            // Receive remaining bits with timeout
            let mut bits_received = 1;
            while bits_received < frame_size
                && running.load(Ordering::Relaxed)
                && !message_complete.load(Ordering::Relaxed)
            {
                match bit_receiver.recv_timeout(std::time::Duration::from_secs(2)) {
                    Ok(bit) => {
                        let _ = frame_bits.push(bit);
                        bits_received += 1;
                    }
                    Err(_) => {
                        // Timeout
                        break;
                    }
                }
            }

            if bits_received != frame_size {
                // Incomplete frame
                frame_errors += 1;
                continue;
            }

            // Process the complete frame - like ESP32C lines 576-620
            match UartFrame::new(frame_bits.clone(), config.uart_format) {
                Ok(frame) => {
                    match extract_char_from_frame(&frame) {
                        Ok((ch, parity_ok)) => {
                            frames_decoded += 1;
                            let _ = received_chars.push(ch);

                            // Track parity errors as frame errors
                            if !parity_ok {
                                frame_errors += 1;
                                log::warn!(
                                    "UART: Frame {} -> char: {:?} (ASCII {}), PARITY ERROR",
                                    frames_decoded,
                                    ch,
                                    ch as u8
                                );
                            } else {
                                log::info!(
                                    "UART: Frame {} -> char: {:?} (ASCII {}), message length: {}",
                                    frames_decoded,
                                    ch,
                                    ch as u8,
                                    received_chars.len()
                                );
                            }

                            // Check for end of message (carriage return)
                            if ch == '\r' {
                                let message: String<256> = received_chars.iter().collect();
                                log::info!(
                                    "UART: Complete message received: '{}'",
                                    message.as_str()
                                );

                                // Store message
                                let mut last_msg = last_message.lock().unwrap();
                                *last_msg = Some(message);

                                // Signal message completion to main task (like nRF line 619)
                                message_complete.store(true, Ordering::Relaxed);
                                log::info!(
                                    "UART: Message complete signal sent, exiting framing task"
                                );

                                received_chars.clear();
                                break; // Exit task after receiving complete message (like nRF)
                            }
                        }
                        Err(e) => {
                            frame_errors += 1;
                            log::warn!(
                                "UART: Frame validation error: {:?}, bits: {:?}",
                                e,
                                frame_bits.as_slice()
                            );
                        }
                    }
                }
                Err(e) => {
                    frame_errors += 1;
                    log::warn!(
                        "UART: Frame creation error: {:?}, {} bits received",
                        e,
                        frame_bits.len()
                    );
                }
            }
        }

        log::info!("UART: Framing task ending (pre-cleanup)");
        log::info!("  Frames decoded: {}", frames_decoded);
        log::info!("  Frame errors: {}", frame_errors);

        // Store frame error count for main task to check
        *frame_error_count.lock().unwrap() = frame_errors;

        if !received_chars.is_empty() {
            log::warn!("  Partial message: {} chars", received_chars.len());
        }

        // Explicitly drop all resources to ensure clean shutdown
        log::info!("UART: Cleaning up resources...");
        drop(bit_receiver);
        drop(last_message);
        drop(message_complete);
        drop(running);
        log::info!("UART: Task cleanup complete");
    }
}
