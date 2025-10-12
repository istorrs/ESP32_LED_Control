/// UART frame format configuration
///
/// Supports common water meter UART formats:
/// - 7E1: 7 data bits, even parity, 1 stop bit (Sensus meters)
/// - 7E2: 7 data bits, even parity, 2 stop bits (Neptune meters)
/// - 8N1: 8 data bits, no parity, 1 stop bit (generic)
/// - 8E1: 8 data bits, even parity, 1 stop bit
/// - 7O1: 7 data bits, odd parity, 1 stop bit
/// - 8N2: 8 data bits, no parity, 2 stop bits

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UartFormat {
    /// 7 data bits, even parity, 1 stop bit (default)
    Format7E1,
    /// 7 data bits, even parity, 2 stop bits
    Format7E2,
    /// 8 data bits, no parity, 1 stop bit
    Format8N1,
    /// 8 data bits, even parity, 1 stop bit
    Format8E1,
    /// 7 data bits, odd parity, 1 stop bit
    Format7O1,
    /// 8 data bits, no parity, 2 stop bits
    Format8N2,
}

impl UartFormat {
    /// Parse UART format from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "7e1" => Some(UartFormat::Format7E1),
            "7e2" => Some(UartFormat::Format7E2),
            "8n1" => Some(UartFormat::Format8N1),
            "8e1" => Some(UartFormat::Format8E1),
            "7o1" => Some(UartFormat::Format7O1),
            "8n2" => Some(UartFormat::Format8N2),
            _ => None,
        }
    }

    /// Convert UART format to string
    pub fn as_str(&self) -> &'static str {
        match self {
            UartFormat::Format7E1 => "7E1",
            UartFormat::Format7E2 => "7E2",
            UartFormat::Format8N1 => "8N1",
            UartFormat::Format8E1 => "8E1",
            UartFormat::Format7O1 => "7O1",
            UartFormat::Format8N2 => "8N2",
        }
    }

    /// Get number of data bits
    pub fn data_bits(&self) -> u8 {
        match self {
            UartFormat::Format7E1 | UartFormat::Format7E2 | UartFormat::Format7O1 => 7,
            UartFormat::Format8N1 | UartFormat::Format8E1 | UartFormat::Format8N2 => 8,
        }
    }

    /// Get parity type
    pub fn parity(&self) -> Parity {
        match self {
            UartFormat::Format7E1 | UartFormat::Format7E2 | UartFormat::Format8E1 => Parity::Even,
            UartFormat::Format7O1 => Parity::Odd,
            UartFormat::Format8N1 | UartFormat::Format8N2 => Parity::None,
        }
    }

    /// Get number of stop bits
    pub fn stop_bits(&self) -> u8 {
        match self {
            UartFormat::Format7E1 | UartFormat::Format8N1 | UartFormat::Format8E1 | UartFormat::Format7O1 => 1,
            UartFormat::Format7E2 | UartFormat::Format8N2 => 2,
        }
    }

    /// Get total bits per frame (start + data + parity + stop)
    pub fn total_bits(&self) -> u8 {
        1 + self.data_bits() + if self.parity() != Parity::None { 1 } else { 0 } + self.stop_bits()
    }
}

impl Default for UartFormat {
    fn default() -> Self {
        UartFormat::Format7E1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

/// Encode a single ASCII character into UART frame bits
///
/// Returns a vector of bits (0 or 1) representing the complete UART frame:
/// [start_bit, data_bits..., parity_bit (if any), stop_bits...]
pub fn encode_uart_frame(ch: u8, format: UartFormat) -> Vec<u8> {
    let mut bits = Vec::new();

    // Start bit (always 0)
    bits.push(0);

    // Data bits (LSB first)
    let data_bits = format.data_bits();
    let data_mask = if data_bits == 7 { 0x7F } else { 0xFF };
    let data = ch & data_mask;

    for i in 0..data_bits {
        bits.push((data >> i) & 1);
    }

    // Parity bit (if needed)
    match format.parity() {
        Parity::Even => {
            // Even parity: count of 1s (including parity bit) should be even
            let ones = data.count_ones() as u8;
            bits.push(ones & 1); // 0 if even number of 1s, 1 if odd
        }
        Parity::Odd => {
            // Odd parity: count of 1s (including parity bit) should be odd
            let ones = data.count_ones() as u8;
            bits.push((ones & 1) ^ 1); // 1 if even number of 1s, 0 if odd
        }
        Parity::None => {
            // No parity bit
        }
    }

    // Stop bits (always 1)
    for _ in 0..format.stop_bits() {
        bits.push(1);
    }

    bits
}

/// Decode UART frame bits into ASCII character
///
/// Returns (character, parity_ok) or None if frame is invalid
pub fn decode_uart_frame(bits: &[u8], format: UartFormat) -> Option<(u8, bool)> {
    let expected_bits = format.total_bits() as usize;
    if bits.len() < expected_bits {
        return None;
    }

    // Check start bit (should be 0)
    if bits[0] != 0 {
        return None;
    }

    // Extract data bits (LSB first)
    let data_bits = format.data_bits();
    let mut data = 0u8;
    for i in 0..data_bits {
        data |= (bits[1 + i as usize]) << i;
    }

    // Check parity (if present)
    let parity_ok = match format.parity() {
        Parity::Even => {
            let parity_bit = bits[1 + data_bits as usize];
            let ones = data.count_ones() as u8;
            ((ones & 1) ^ parity_bit) == 0
        }
        Parity::Odd => {
            let parity_bit = bits[1 + data_bits as usize];
            let ones = data.count_ones() as u8;
            ((ones & 1) ^ parity_bit) == 1
        }
        Parity::None => true, // No parity to check
    };

    // Check stop bits (should all be 1)
    let stop_bit_start = 1 + data_bits as usize + if format.parity() != Parity::None { 1 } else { 0 };
    for i in 0..format.stop_bits() {
        if bits[stop_bit_start + i as usize] != 1 {
            return None; // Invalid stop bit
        }
    }

    Some((data, parity_ok))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_7e1_properties() {
        let format = UartFormat::Format7E1;
        assert_eq!(format.data_bits(), 7);
        assert_eq!(format.parity(), Parity::Even);
        assert_eq!(format.stop_bits(), 1);
        assert_eq!(format.total_bits(), 10); // 1 start + 7 data + 1 parity + 1 stop
    }

    #[test]
    fn test_format_8n1_properties() {
        let format = UartFormat::Format8N1;
        assert_eq!(format.data_bits(), 8);
        assert_eq!(format.parity(), Parity::None);
        assert_eq!(format.stop_bits(), 1);
        assert_eq!(format.total_bits(), 10); // 1 start + 8 data + 0 parity + 1 stop
    }

    #[test]
    fn test_encode_decode_7e1() {
        let format = UartFormat::Format7E1;
        let ch = b'V'; // 0x56 = 0b1010110

        let bits = encode_uart_frame(ch, format);
        assert_eq!(bits.len(), 10);
        assert_eq!(bits[0], 0); // start bit

        let (decoded, parity_ok) = decode_uart_frame(&bits, format).unwrap();
        assert_eq!(decoded, ch & 0x7F);
        assert!(parity_ok);
    }

    #[test]
    fn test_encode_decode_8n1() {
        let format = UartFormat::Format8N1;
        let ch = b'A'; // 0x41

        let bits = encode_uart_frame(ch, format);
        assert_eq!(bits.len(), 10);

        let (decoded, parity_ok) = decode_uart_frame(&bits, format).unwrap();
        assert_eq!(decoded, ch);
        assert!(parity_ok); // Always true for no parity
    }

    #[test]
    fn test_format_from_str() {
        assert_eq!(UartFormat::from_str("7e1"), Some(UartFormat::Format7E1));
        assert_eq!(UartFormat::from_str("7E1"), Some(UartFormat::Format7E1));
        assert_eq!(UartFormat::from_str("8N1"), Some(UartFormat::Format8N1));
        assert_eq!(UartFormat::from_str("invalid"), None);
    }
}
