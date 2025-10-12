use crate::uart_format::UartFormat;
use heapless::String;

#[derive(Debug, Clone)]
pub struct MtuConfig {
    /// Baud rate for communication
    pub baud_rate: u32,

    /// Power-up delay before starting clock cycles (ms)
    pub power_up_delay_ms: u64,

    /// Bit timeout for incomplete frames (ms)
    pub bit_timeout_ms: u64,

    /// Maximum runtime for MTU operation (seconds)
    pub runtime_secs: u64,

    /// UART framing configuration (7E1, 7E2, 8N1, 8E1, 7O1, 8N2)
    pub uart_format: UartFormat,

    /// Expected message for testing (default is meter's default response)
    pub expected_message: String<256>,

    /// Running count of successful message reads
    pub successful_reads: u32,

    /// Running count of corrupted/failed message reads
    pub corrupted_reads: u32,
}

// Keep old UartFraming enum for backwards compatibility, but mark as deprecated
#[deprecated(since = "0.2.0", note = "Use UartFormat instead")]
#[derive(Debug, Clone, Copy)]
pub enum UartFraming {
    /// 7 data bits, even parity, 1 stop bit (Sensus Standard)
    SevenE1,
    /// 7 data bits, even parity, 2 stop bits (Neptune)
    SevenE2,
}

#[allow(deprecated)]
impl UartFraming {
    pub fn bits_per_frame(self) -> usize {
        match self {
            UartFraming::SevenE1 => 10, // 1 start + 7 data + 1 parity + 1 stop
            UartFraming::SevenE2 => 11, // 1 start + 7 data + 1 parity + 2 stop
        }
    }
}

impl MtuConfig {
    /// Calculate bit duration in microseconds from baud rate
    pub fn bit_duration_micros(&self) -> u64 {
        1_000_000 / self.baud_rate as u64
    }

    /// Calculate bit duration in milliseconds
    pub fn bit_duration_millis(&self) -> u64 {
        1_000 / self.baud_rate as u64
    }
}

impl Default for MtuConfig {
    fn default() -> Self {
        let mut expected_message = String::new();
        // Default expected message matches meter's default response
        let _ = expected_message.push_str(
            "V;RB00000200;IB61564400;A1000;Z3214;XT0746;MT0683;RR00000000;GX000000;GN000000\r",
        );

        Self {
            baud_rate: 1200,       // Default to 1200 baud
            power_up_delay_ms: 10, // Very short delay to be ready before meter starts
            bit_timeout_ms: 2000,
            runtime_secs: 30,
            uart_format: UartFormat::Format7E1, // Sensus Standard default (7E1)
            expected_message,
            successful_reads: 0,
            corrupted_reads: 0,
        }
    }
}
