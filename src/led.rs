use esp_idf_hal::gpio::{Output, Pin, PinDriver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// LED pulse configuration with validation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PulseConfig {
    /// Duration LED stays ON in milliseconds (1-2000ms)
    pub duration_ms: u32,
    /// Period between pulse starts in milliseconds (3000-3600000ms = 1 hour)
    pub period_ms: u32,
}

impl PulseConfig {
    /// Minimum pulse duration (1ms)
    pub const MIN_DURATION_MS: u32 = 1;
    /// Maximum pulse duration (2 seconds)
    pub const MAX_DURATION_MS: u32 = 2000;
    /// Minimum period (3 seconds)
    pub const MIN_PERIOD_MS: u32 = 3000;
    /// Maximum period (1 hour)
    pub const MAX_PERIOD_MS: u32 = 3_600_000;
    /// Default configuration (500ms / 5s)
    pub const DEFAULT: PulseConfig = PulseConfig {
        duration_ms: 500,
        period_ms: 5000,
    };

    /// Create new pulse configuration with validation
    pub fn new(duration_ms: u32, period_ms: u32) -> Result<Self, String> {
        if duration_ms < Self::MIN_DURATION_MS || duration_ms > Self::MAX_DURATION_MS {
            return Err(format!(
                "Duration must be between {}ms and {}ms",
                Self::MIN_DURATION_MS,
                Self::MAX_DURATION_MS
            ));
        }
        if period_ms < Self::MIN_PERIOD_MS || period_ms > Self::MAX_PERIOD_MS {
            return Err(format!(
                "Period must be between {}ms and {}ms",
                Self::MIN_PERIOD_MS,
                Self::MAX_PERIOD_MS
            ));
        }
        if duration_ms >= period_ms {
            return Err(format!(
                "Duration ({}ms) must be less than period ({}ms)",
                duration_ms, period_ms
            ));
        }
        Ok(PulseConfig {
            duration_ms,
            period_ms,
        })
    }
}

/// LED status patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedStatus {
    /// LED off - Idle state
    Off,
    /// Solid on - Always on
    SolidOn,
    /// Custom pulse pattern with configurable timing
    CustomPulse(PulseConfig),
    /// Slow blink (1 Hz) - For backwards compatibility
    SlowBlink,
    /// Fast blink (5 Hz) - Error state
    FastBlink,
}

impl Default for LedStatus {
    fn default() -> Self {
        LedStatus::CustomPulse(PulseConfig::DEFAULT)
    }
}

/// LED Manager - Controls a single LED with different patterns
pub struct LedManager {
    status: Arc<Mutex<LedStatus>>,
}

impl LedManager {
    /// Create new LED manager and spawn background task
    /// Starts with default pulse configuration (500ms / 5s)
    pub fn new<P: Pin + 'static>(led_pin: PinDriver<'static, P, Output>) -> Self {
        let status = Arc::new(Mutex::new(LedStatus::default()));
        let status_clone = status.clone();

        // Spawn LED control task
        thread::Builder::new()
            .stack_size(4096)
            .name("led_task".to_string())
            .spawn(move || {
                Self::led_task(led_pin, status_clone);
            })
            .expect("Failed to spawn LED task");

        Self { status }
    }

    /// Set LED status
    pub fn set_status(&self, new_status: LedStatus) {
        let mut status = self.status.lock().unwrap();
        if *status != new_status {
            log::info!("LED: Status changed to {:?}", new_status);
            *status = new_status;
        }
    }

    /// Set custom pulse configuration
    pub fn set_pulse(&self, config: PulseConfig) {
        self.set_status(LedStatus::CustomPulse(config));
    }

    /// Turn LED on
    pub fn turn_on(&self) {
        self.set_status(LedStatus::SolidOn);
    }

    /// Turn LED off
    pub fn turn_off(&self) {
        self.set_status(LedStatus::Off);
    }

    /// Set blink rate in Hz (1-10 Hz)
    pub fn set_blink(&self, frequency_hz: u32) {
        match frequency_hz {
            0 => self.set_status(LedStatus::Off),
            1 => self.set_status(LedStatus::SlowBlink),
            5 => self.set_status(LedStatus::FastBlink),
            _ => {
                // Convert frequency to period (with 50% duty cycle)
                let period_ms = 1000 / frequency_hz;
                let duration_ms = period_ms / 2;

                // Clamp to valid ranges
                let duration_ms = duration_ms.max(PulseConfig::MIN_DURATION_MS).min(PulseConfig::MAX_DURATION_MS);
                let period_ms = (period_ms * 2).max(PulseConfig::MIN_PERIOD_MS).min(PulseConfig::MAX_PERIOD_MS);

                if let Ok(config) = PulseConfig::new(duration_ms, period_ms) {
                    self.set_pulse(config);
                } else {
                    log::warn!("LED: Invalid blink frequency {}Hz, using SlowBlink", frequency_hz);
                    self.set_status(LedStatus::SlowBlink);
                }
            }
        }
    }

    /// Get current LED status
    pub fn get_status(&self) -> LedStatus {
        *self.status.lock().unwrap()
    }

    /// Get current pulse configuration if in CustomPulse mode
    pub fn get_pulse_config(&self) -> Option<PulseConfig> {
        match self.get_status() {
            LedStatus::CustomPulse(config) => Some(config),
            _ => None,
        }
    }

    /// LED control task - runs in background thread
    fn led_task<P: Pin>(mut led_pin: PinDriver<'static, P, Output>, status: Arc<Mutex<LedStatus>>) {
        log::info!("LED: Task started with default pulse (500ms / 5s)");

        let mut led_state = false;
        let mut cycle_start = std::time::Instant::now();

        loop {
            let current_status = *status.lock().unwrap();

            match current_status {
                LedStatus::Off => {
                    if led_state {
                        let _ = led_pin.set_low();
                        led_state = false;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                LedStatus::SolidOn => {
                    if !led_state {
                        let _ = led_pin.set_high();
                        led_state = true;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                LedStatus::CustomPulse(config) => {
                    let elapsed = cycle_start.elapsed().as_millis() as u32;

                    if elapsed < config.duration_ms {
                        // ON phase
                        if !led_state {
                            let _ = led_pin.set_high();
                            led_state = true;
                        }
                    } else if elapsed < config.period_ms {
                        // OFF phase
                        if led_state {
                            let _ = led_pin.set_low();
                            led_state = false;
                        }
                    } else {
                        // Start new cycle
                        cycle_start = std::time::Instant::now();
                        let _ = led_pin.set_high();
                        led_state = true;
                    }

                    // Sleep for a reasonable polling interval
                    // Use shorter sleep for short pulses, longer for long periods
                    let sleep_ms = if config.duration_ms < 100 {
                        10
                    } else if config.period_ms > 10000 {
                        100
                    } else {
                        50
                    };
                    thread::sleep(Duration::from_millis(sleep_ms));
                }
                LedStatus::SlowBlink => {
                    // 1 Hz = 500ms on, 500ms off
                    let elapsed = cycle_start.elapsed().as_millis() as u32;
                    if elapsed >= 1000 {
                        cycle_start = std::time::Instant::now();
                    }

                    let new_state = elapsed < 500;
                    if new_state != led_state {
                        if new_state {
                            let _ = led_pin.set_high();
                        } else {
                            let _ = led_pin.set_low();
                        }
                        led_state = new_state;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                LedStatus::FastBlink => {
                    // 5 Hz = 100ms on, 100ms off
                    let elapsed = cycle_start.elapsed().as_millis() as u32;
                    if elapsed >= 200 {
                        cycle_start = std::time::Instant::now();
                    }

                    let new_state = elapsed < 100;
                    if new_state != led_state {
                        if new_state {
                            let _ = led_pin.set_high();
                        } else {
                            let _ = led_pin.set_low();
                        }
                        led_state = new_state;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
}

// SAFETY: LedManager only holds Arc<Mutex<>> which is Send+Sync
unsafe impl Send for LedManager {}
unsafe impl Sync for LedManager {}
