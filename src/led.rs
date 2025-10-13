use esp_idf_hal::gpio::{Output, Pin, PinDriver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// LED status patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedStatus {
    /// LED off - Idle state
    Off,
    /// Slow blink (1 Hz) - WiFi/MQTT operations
    SlowBlink,
    /// Solid on - MTU reading in progress
    SolidOn,
    /// Fast blink (5 Hz) - Error state
    FastBlink,
}

/// LED Manager - Controls a single LED with different patterns
pub struct LedManager {
    status: Arc<Mutex<LedStatus>>,
}

impl LedManager {
    /// Create new LED manager and spawn background task
    pub fn new<P: Pin + 'static>(led_pin: PinDriver<'static, P, Output>) -> Self {
        let status = Arc::new(Mutex::new(LedStatus::Off));
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

    /// Get current LED status
    pub fn get_status(&self) -> LedStatus {
        *self.status.lock().unwrap()
    }

    /// LED control task - runs in background thread
    fn led_task<P: Pin>(mut led_pin: PinDriver<'static, P, Output>, status: Arc<Mutex<LedStatus>>) {
        log::info!("LED: Task started");

        let mut led_state = false;
        let mut last_toggle = std::time::Instant::now();

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
                LedStatus::SlowBlink => {
                    // 1 Hz = 500ms on, 500ms off
                    if last_toggle.elapsed() >= Duration::from_millis(500) {
                        led_state = !led_state;
                        if led_state {
                            let _ = led_pin.set_high();
                        } else {
                            let _ = led_pin.set_low();
                        }
                        last_toggle = std::time::Instant::now();
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                LedStatus::FastBlink => {
                    // 5 Hz = 100ms on, 100ms off
                    if last_toggle.elapsed() >= Duration::from_millis(100) {
                        led_state = !led_state;
                        if led_state {
                            let _ = led_pin.set_high();
                        } else {
                            let _ = led_pin.set_low();
                        }
                        last_toggle = std::time::Instant::now();
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
