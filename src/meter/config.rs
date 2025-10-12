use crate::uart_format::UartFormat;
use heapless::String;

#[derive(Debug, Clone, Copy)]
pub enum MeterType {
    Sensus,
    Neptune,
}

impl MeterType {
    /// Get default UART format for this meter type
    pub fn default_format(&self) -> UartFormat {
        match self {
            MeterType::Sensus => UartFormat::Format7E1,
            MeterType::Neptune => UartFormat::Format7E2,
        }
    }

    // Keep old framing() method for backwards compatibility
    #[deprecated(since = "0.2.0", note = "Use default_format() instead")]
    #[allow(deprecated)]
    pub fn framing(&self) -> crate::mtu::UartFraming {
        match self {
            MeterType::Sensus => crate::mtu::UartFraming::SevenE1,
            MeterType::Neptune => crate::mtu::UartFraming::SevenE2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MeterConfig {
    pub meter_type: MeterType,
    /// UART frame format (7E1, 7E2, 8N1, 8E1, 7O1, 8N2)
    pub uart_format: UartFormat,
    pub response_message: String<256>,
    pub response_delay_ms: u64,
    pub enabled: bool,
}

impl Default for MeterConfig {
    fn default() -> Self {
        let mut default_message = String::new();
        // Realistic water meter response message
        let _ = default_message.push_str(
            "V;RB00000200;IB61564400;A1000;Z3214;XT0746;MT0683;RR00000000;GX000000;GN000000\r",
        );

        let meter_type = MeterType::Sensus;
        Self {
            meter_type,
            uart_format: meter_type.default_format(), // Default to Sensus 7E1
            response_message: default_message,
            response_delay_ms: 50,
            enabled: true,
        }
    }
}
