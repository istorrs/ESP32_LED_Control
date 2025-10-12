use super::error::{MtuError, MtuResult};
use crate::uart_format::{decode_uart_frame, UartFormat};
use heapless::Vec;

#[derive(Debug, Clone)]
pub struct UartFrame {
    pub bits: Vec<u8, 16>, // Max 16 bits per frame
    pub format: UartFormat,
}

impl UartFrame {
    pub fn new(bits: Vec<u8, 16>, format: UartFormat) -> MtuResult<Self> {
        let expected_bits = format.total_bits() as usize;
        if bits.len() != expected_bits {
            return Err(MtuError::FramingError);
        }
        Ok(Self { bits, format })
    }

    pub fn validate(&self) -> MtuResult<(u8, bool)> {
        let expected_bits = self.format.total_bits() as usize;
        if self.bits.len() != expected_bits {
            return Err(MtuError::FramingErrorInvalidBitCount);
        }

        // Use the new decode_uart_frame function
        match decode_uart_frame(self.bits.as_slice(), self.format) {
            Some((data, parity_ok)) => Ok((data, parity_ok)),
            None => Err(MtuError::FramingError),
        }
    }
}

pub fn extract_char_from_frame(frame: &UartFrame) -> MtuResult<(char, bool)> {
    let (data, parity_ok) = frame.validate()?;

    // Convert to ASCII character
    if data <= 127 {
        Ok((data as char, parity_ok))
    } else {
        Err(MtuError::FramingError)
    }
}

pub fn bits_to_frame(bits: &[u8], format: UartFormat) -> MtuResult<UartFrame> {
    let mut frame_bits: Vec<u8, 16> = Vec::new();

    for &bit in bits {
        if frame_bits.push(bit).is_err() {
            return Err(MtuError::FramingError);
        }
    }

    UartFrame::new(frame_bits, format)
}
