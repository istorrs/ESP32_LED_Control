use esp_idf_hal::gpio::OutputPin;
use esp_idf_hal::ledc::{LedcDriver, LedcTimerDriver, Resolution};
use esp_idf_hal::peripheral::Peripheral;
use esp_idf_hal::timer::{TimerConfig, TimerDriver};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// LED pulse configuration with validation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PulseConfig {
    /// Duration LED stays ON in microseconds (100us-2s)
    pub duration_us: u32,
    /// Period between pulse starts in microseconds (500us-1h)
    pub period_us: u32,
    /// LED brightness percentage (0-100%)
    pub brightness_percent: u8,
}

impl PulseConfig {
    /// Minimum pulse duration (100us)
    pub const MIN_DURATION_US: u32 = 100;
    /// Maximum pulse duration (2 seconds)
    pub const MAX_DURATION_US: u32 = 2_000_000;
    /// Minimum period (500us)
    pub const MIN_PERIOD_US: u32 = 500;
    /// Maximum period (1 hour)
    pub const MAX_PERIOD_US: u32 = 3_600_000_000;
    /// Default brightness (75%)
    pub const DEFAULT_BRIGHTNESS: u8 = 75;
    /// Default configuration (500ms / 5s @ 75% brightness)
    pub const DEFAULT: PulseConfig = PulseConfig {
        duration_us: 500_000,
        period_us: 5_000_000,
        brightness_percent: Self::DEFAULT_BRIGHTNESS,
    };

    /// Create new pulse configuration with validation (microseconds)
    pub fn new(duration_us: u32, period_us: u32, brightness_percent: u8) -> Result<Self, String> {
        if duration_us < Self::MIN_DURATION_US || duration_us > Self::MAX_DURATION_US {
            return Err(format!(
                "Duration must be between {}us and {}us ({}ms-{}s)",
                Self::MIN_DURATION_US,
                Self::MAX_DURATION_US,
                Self::MIN_DURATION_US / 1000,
                Self::MAX_DURATION_US / 1_000_000
            ));
        }
        if period_us < Self::MIN_PERIOD_US || period_us > Self::MAX_PERIOD_US {
            return Err(format!(
                "Period must be between {}us and {}us ({}ms-{}h)",
                Self::MIN_PERIOD_US,
                Self::MAX_PERIOD_US,
                Self::MIN_PERIOD_US / 1000,
                Self::MAX_PERIOD_US / 3_600_000_000
            ));
        }
        if duration_us >= period_us {
            return Err(format!(
                "Duration ({}us) must be less than period ({}us)",
                duration_us, period_us
            ));
        }
        if brightness_percent > 100 {
            return Err("Brightness must be between 0 and 100%".to_string());
        }
        Ok(PulseConfig {
            duration_us,
            period_us,
            brightness_percent,
        })
    }

    /// Helper: Create from milliseconds (for backward compatibility)
    pub fn new_ms(duration_ms: u32, period_ms: u32, brightness_percent: u8) -> Result<Self, String> {
        Self::new(duration_ms * 1000, period_ms * 1000, brightness_percent)
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
    /// Slow blink (1 Hz)
    SlowBlink,
    /// Fast blink (5 Hz)
    FastBlink,
}

impl Default for LedStatus {
    fn default() -> Self {
        LedStatus::CustomPulse(PulseConfig::DEFAULT)
    }
}

/// LED statistics for reporting
#[derive(Debug, Clone)]
pub struct LedStatistics {
    /// Number of pulses since last config change
    pub pulse_count: u64,
    /// Timestamp of first pulse after config change
    pub first_pulse_time: Option<Instant>,
    /// Timestamp of last pulse start
    pub last_pulse_time: Option<Instant>,
    /// Timestamp of last config change
    pub config_changed_time: Instant,
    /// Total accumulated ON time in microseconds
    pub total_on_time_us: u64,
    /// Total accumulated OFF time in microseconds
    pub total_off_time_us: u64,
    /// Timestamp when we last transitioned (for calculating durations)
    pub last_transition_time: Option<Instant>,
    /// Whether LED was ON at last transition
    pub was_on_at_last_transition: bool,
}

impl Default for LedStatistics {
    fn default() -> Self {
        Self {
            pulse_count: 0,
            first_pulse_time: None,
            last_pulse_time: None,
            config_changed_time: Instant::now(),
            total_on_time_us: 0,
            total_off_time_us: 0,
            last_transition_time: None,
            was_on_at_last_transition: false,
        }
    }
}

/// LED Manager - Controls a single LED with different patterns using hardware timer
pub struct LedManager {
    status: Arc<Mutex<LedStatus>>,
    statistics: Arc<Mutex<LedStatistics>>,
    ledc_driver: Arc<Mutex<LedcDriver<'static>>>,
    // Keep timer alive - it must not be dropped!
    #[allow(dead_code)]
    timer_driver: TimerDriver<'static>,
    // ISR state - all atomic for lock-free access
    pulse_phase: Arc<AtomicBool>,
    pulse_counter: Arc<AtomicU32>,
    duration_us: Arc<AtomicU32>,
    period_us: Arc<AtomicU32>,
    brightness: Arc<AtomicU8>,
}

impl LedManager {
    /// Create new LED manager using hardware timer for microsecond precision
    pub fn new<C, T, P, HWT>(
        ledc_channel: impl Peripheral<P = C> + 'static,
        ledc_timer: impl Peripheral<P = T> + 'static,
        led_pin: impl Peripheral<P = P> + 'static,
        hw_timer: impl Peripheral<P = HWT> + 'static,
    ) -> anyhow::Result<Self>
    where
        C: esp_idf_hal::ledc::LedcChannel + 'static,
        T: esp_idf_hal::ledc::LedcTimer<SpeedMode = C::SpeedMode> + 'static,
        P: OutputPin + 'static,
        HWT: esp_idf_hal::timer::Timer + 'static,
    {
        let status = Arc::new(Mutex::new(LedStatus::default()));
        let statistics = Arc::new(Mutex::new(LedStatistics::default()));

        // ISR state atomics
        let pulse_phase = Arc::new(AtomicBool::new(true)); // Start in ON phase
        let pulse_counter = Arc::new(AtomicU32::new(0));
        let default_config = PulseConfig::DEFAULT;
        let duration_us = Arc::new(AtomicU32::new(default_config.duration_us));
        let period_us = Arc::new(AtomicU32::new(default_config.period_us));
        let brightness = Arc::new(AtomicU8::new(default_config.brightness_percent));

        // Configure LEDC for PWM brightness control
        let ledc_timer_driver = LedcTimerDriver::new(
            ledc_timer,
            &esp_idf_hal::ledc::config::TimerConfig::default()
                .frequency(5000.into())
                .resolution(Resolution::Bits13),
        )?;

        let mut ledc_driver = LedcDriver::new(ledc_channel, ledc_timer_driver, led_pin)?;

        // Start with LED ON at default brightness
        ledc_driver.set_duty(Self::brightness_to_duty(default_config.brightness_percent))?;

        let ledc_driver: LedcDriver<'static> = unsafe { std::mem::transmute(ledc_driver) };
        let ledc_driver = Arc::new(Mutex::new(ledc_driver));

        // Configure hardware timer: 1MHz tick rate (1μs per tick)
        let timer_config = TimerConfig::new()
            .divider(80)        // 80MHz / 80 = 1MHz
            .auto_reload(true);

        let mut timer: TimerDriver<'static> = unsafe {
            std::mem::transmute(TimerDriver::new(hw_timer, &timer_config)?)
        };

        // Clone Arcs for ISR closure
        let ledc_for_isr = ledc_driver.clone();
        let phase_for_isr = pulse_phase.clone();
        let counter_for_isr = pulse_counter.clone();
        let dur_for_isr = duration_us.clone();
        let period_for_isr = period_us.clone();
        let bright_for_isr = brightness.clone();
        let stats_for_isr = statistics.clone();

        // Set up timer alarm for 100μs intervals (10kHz tick rate)
        timer.set_counter(0)?;
        timer.set_alarm(100)?; // 100μs intervals

        // Subscribe ISR callback
        unsafe {
            timer.subscribe(move || {
                // Load atomic config
                let dur = dur_for_isr.load(Ordering::Relaxed);
                let period = period_for_isr.load(Ordering::Relaxed);
                let bright = bright_for_isr.load(Ordering::Relaxed);

                // Special cases: don't pulse
                if dur == 0 || dur == u32::MAX {
                    return;
                }

                // Get current state
                let counter = counter_for_isr.load(Ordering::Relaxed);
                let is_on = phase_for_isr.load(Ordering::Relaxed);

                // Try to get LEDC - skip if locked
                if let Ok(mut ledc) = ledc_for_isr.try_lock() {
                    // Check if we should transition based on current counter value
                    if is_on && counter >= dur {
                        // Turn OFF - accumulate ON time
                        phase_for_isr.store(false, Ordering::Relaxed);
                        let _ = ledc.set_duty(0);

                        // Track ON->OFF transition
                        if let Ok(mut stats) = stats_for_isr.try_lock() {
                            let now = Instant::now();
                            if let Some(last_transition) = stats.last_transition_time {
                                if stats.was_on_at_last_transition {
                                    // We were ON, now turning OFF - accumulate ON time
                                    let duration_us = now.duration_since(last_transition).as_micros() as u64;
                                    stats.total_on_time_us += duration_us;
                                }
                            }
                            stats.last_transition_time = Some(now);
                            stats.was_on_at_last_transition = false;
                        }
                    } else if !is_on && counter >= period {
                        // Start new cycle - Turn ON
                        counter_for_isr.store(0, Ordering::Relaxed);
                        phase_for_isr.store(true, Ordering::Relaxed);
                        let duty = (8191u32 * bright as u32) / 100;
                        let _ = ledc.set_duty(duty);

                        // Track OFF->ON transition and update pulse stats
                        if let Ok(mut stats) = stats_for_isr.try_lock() {
                            let now = Instant::now();

                            // Accumulate OFF time from previous transition
                            if let Some(last_transition) = stats.last_transition_time {
                                if !stats.was_on_at_last_transition {
                                    // We were OFF, now turning ON - accumulate OFF time
                                    let duration_us = now.duration_since(last_transition).as_micros() as u64;
                                    stats.total_off_time_us += duration_us;
                                }
                            }

                            // Update pulse count
                            stats.pulse_count += 1;
                            if stats.first_pulse_time.is_none() {
                                stats.first_pulse_time = Some(now);
                            }
                            stats.last_pulse_time = Some(now);

                            stats.last_transition_time = Some(now);
                            stats.was_on_at_last_transition = true;
                        }
                    }
                }

                // Increment counter AFTER all checks are done
                counter_for_isr.fetch_add(100, Ordering::Relaxed);
            })?;
        }

        // Enable alarm and interrupts, then start timer
        timer.enable_alarm(true)?;
        timer.enable_interrupt()?;
        timer.enable(true)?;

        log::info!("LED: Hardware timer initialized (100μs resolution)");

        Ok(Self {
            status,
            statistics,
            ledc_driver,
            timer_driver: timer,
            pulse_phase,
            pulse_counter,
            duration_us,
            period_us,
            brightness,
        })
    }

    /// Set LED status
    pub fn set_status(&self, new_status: LedStatus) {
        let mut status = self.status.lock().unwrap();
        if *status != new_status {
            log::info!("LED: Status changed to {:?}", new_status);
            *status = new_status;

            // Update atomics for ISR
            match new_status {
                LedStatus::Off => {
                    self.duration_us.store(0, Ordering::Relaxed);
                    self.period_us.store(1, Ordering::Relaxed);
                    self.brightness.store(0, Ordering::Relaxed);
                }
                LedStatus::SolidOn => {
                    self.duration_us.store(u32::MAX, Ordering::Relaxed);
                    self.period_us.store(u32::MAX, Ordering::Relaxed);
                    self.brightness.store(100, Ordering::Relaxed);
                }
                LedStatus::CustomPulse(config) => {
                    self.duration_us.store(config.duration_us, Ordering::Relaxed);
                    self.period_us.store(config.period_us, Ordering::Relaxed);
                    self.brightness.store(config.brightness_percent, Ordering::Relaxed);
                }
                LedStatus::SlowBlink => {
                    self.duration_us.store(500_000, Ordering::Relaxed);
                    self.period_us.store(1_000_000, Ordering::Relaxed);
                    self.brightness.store(100, Ordering::Relaxed);
                }
                LedStatus::FastBlink => {
                    self.duration_us.store(100_000, Ordering::Relaxed);
                    self.period_us.store(200_000, Ordering::Relaxed);
                    self.brightness.store(100, Ordering::Relaxed);
                }
            }

            // Reset phase and counter
            self.pulse_counter.store(0, Ordering::Relaxed);
            self.pulse_phase.store(true, Ordering::Relaxed);

            // Immediately update LED
            if let Ok(mut ledc) = self.ledc_driver.lock() {
                match new_status {
                    LedStatus::Off => {
                        let _ = ledc.set_duty(0);
                    }
                    LedStatus::SolidOn => {
                        let _ = ledc.set_duty(Self::brightness_to_duty(100));
                    }
                    LedStatus::CustomPulse(config) => {
                        let _ = ledc.set_duty(Self::brightness_to_duty(config.brightness_percent));
                    }
                    LedStatus::SlowBlink | LedStatus::FastBlink => {
                        let _ = ledc.set_duty(Self::brightness_to_duty(100));
                    }
                }
            }

            // Reset statistics
            let mut stats = self.statistics.lock().unwrap();
            stats.pulse_count = 0;
            stats.first_pulse_time = None;
            stats.last_pulse_time = None;
            stats.config_changed_time = Instant::now();
            stats.total_on_time_us = 0;
            stats.total_off_time_us = 0;
            stats.last_transition_time = Some(Instant::now());
            // Set initial state based on new status
            stats.was_on_at_last_transition = !matches!(new_status, LedStatus::Off);
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
                let period_us = 1_000_000 / frequency_hz;
                let duration_us = period_us / 2;
                let duration_us = duration_us.max(PulseConfig::MIN_DURATION_US).min(PulseConfig::MAX_DURATION_US);
                let period_us = (period_us * 2).max(PulseConfig::MIN_PERIOD_US).min(PulseConfig::MAX_PERIOD_US);

                if let Ok(config) = PulseConfig::new(duration_us, period_us, 100) {
                    self.set_pulse(config);
                } else {
                    log::warn!("LED: Invalid blink frequency {}Hz", frequency_hz);
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

    /// Get LED statistics
    pub fn get_statistics(&self) -> LedStatistics {
        self.statistics.lock().unwrap().clone()
    }

    /// Convert brightness percentage (0-100) to LEDC duty cycle (0-8191 for 13-bit)
    fn brightness_to_duty(brightness_percent: u8) -> u32 {
        const MAX_DUTY: u32 = 8191;
        (MAX_DUTY * brightness_percent as u32) / 100
    }
}

// SAFETY: LedManager only holds Arc<Mutex<>> which is Send+Sync
unsafe impl Send for LedManager {}
unsafe impl Sync for LedManager {}
