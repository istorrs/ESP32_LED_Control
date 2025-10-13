use super::{parser::CommandParser, CliError, CLI_BUFFER_SIZE};
use esp_idf_hal::uart::{UartRxDriver, UartTxDriver};

const HISTORY_SIZE: usize = 10;

pub struct Terminal<'d> {
    pub uart_tx: UartTxDriver<'d>,
    pub uart_rx: UartRxDriver<'d>,
    line_buffer: String,
    cursor_pos: usize,
    command_history: Vec<String>,
    history_index: Option<usize>,
    escape_state: EscapeState,
}

#[derive(Clone, Copy, PartialEq)]
enum EscapeState {
    Normal,
    Escape,
    Csi,
}

impl<'d> Terminal<'d> {
    pub fn new(uart_tx: UartTxDriver<'d>, uart_rx: UartRxDriver<'d>) -> Self {
        Self {
            uart_tx,
            uart_rx,
            line_buffer: String::new(),
            cursor_pos: 0,
            command_history: Vec::new(),
            history_index: None,
            escape_state: EscapeState::Normal,
        }
    }

    pub fn write_str(&mut self, s: &str) -> Result<(), CliError> {
        self.uart_tx
            .write(s.as_bytes())
            .map_err(|_| CliError::UartError)?;
        Ok(())
    }

    pub fn write_line(&mut self, s: &str) -> Result<(), CliError> {
        self.write_str(s)?;
        self.write_str("\r\n")
    }

    pub fn print_prompt(&mut self) -> Result<(), CliError> {
        self.write_str("ESP32 CLI> ")
    }

    pub fn read_char(&mut self) -> Result<Option<u8>, CliError> {
        let mut buf = [0u8; 1];
        match self.uart_rx.read(&mut buf, 0) {
            Ok(1) => Ok(Some(buf[0])),
            Ok(0) => Ok(None),
            Ok(_) => Ok(None),
            Err(_) => Err(CliError::UartError),
        }
    }

    pub fn handle_char(&mut self, ch: u8) -> Result<Option<String>, CliError> {
        match self.escape_state {
            EscapeState::Normal => match ch {
                b'\r' | b'\n' => {
                    // Enter pressed - return the command
                    self.write_str("\r\n")?;
                    let command = self.line_buffer.clone();

                    // Add to history if non-empty and different from last entry
                    if !command.is_empty() {
                        let should_add = self.command_history.is_empty()
                            || self.command_history.last() != Some(&command);

                        if should_add {
                            if self.command_history.len() >= HISTORY_SIZE {
                                self.command_history.remove(0);
                            }
                            self.command_history.push(command.clone());
                        }
                    }

                    self.line_buffer.clear();
                    self.cursor_pos = 0;
                    self.history_index = None;
                    Ok(Some(command))
                }
                b'\x1b' => {
                    // ESC - start escape sequence
                    self.escape_state = EscapeState::Escape;
                    Ok(None)
                }
                b'\x08' | b'\x7f' => {
                    // Backspace
                    if !self.line_buffer.is_empty() && self.cursor_pos > 0 {
                        self.delete_char_before_cursor()?;
                    }
                    Ok(None)
                }
                b'\t' => {
                    // Tab - autocomplete
                    self.handle_tab_completion()?;
                    Ok(None)
                }
                0x20..=0x7E => {
                    // Printable ASCII character
                    if self.line_buffer.len() < CLI_BUFFER_SIZE - 1 {
                        self.insert_char_at_cursor(ch as char)?;
                    }
                    Ok(None)
                }
                _ => {
                    // Ignore other control characters
                    Ok(None)
                }
            },
            EscapeState::Escape => {
                match ch {
                    b'[' => {
                        // ESC[ - Control Sequence Introducer
                        self.escape_state = EscapeState::Csi;
                        Ok(None)
                    }
                    _ => {
                        // Unknown escape sequence, reset to normal
                        self.escape_state = EscapeState::Normal;
                        Ok(None)
                    }
                }
            }
            EscapeState::Csi => {
                match ch {
                    b'A' => {
                        // Up arrow - previous command in history
                        self.handle_history_up()?;
                        self.escape_state = EscapeState::Normal;
                        Ok(None)
                    }
                    b'B' => {
                        // Down arrow - next command in history
                        self.handle_history_down()?;
                        self.escape_state = EscapeState::Normal;
                        Ok(None)
                    }
                    b'C' => {
                        // Right arrow - move cursor right
                        self.handle_cursor_right()?;
                        self.escape_state = EscapeState::Normal;
                        Ok(None)
                    }
                    b'D' => {
                        // Left arrow - move cursor left
                        self.handle_cursor_left()?;
                        self.escape_state = EscapeState::Normal;
                        Ok(None)
                    }
                    _ => {
                        // Other CSI sequences, ignore for now
                        self.escape_state = EscapeState::Normal;
                        Ok(None)
                    }
                }
            }
        }
    }

    pub fn clear_screen(&mut self) -> Result<(), CliError> {
        // ANSI escape sequence to clear screen and move cursor to top
        self.write_str("\x1b[2J\x1b[H")
    }

    fn handle_tab_completion(&mut self) -> Result<(), CliError> {
        let current_line = self.line_buffer.clone();
        let words: Vec<&str> = current_line.split_whitespace().collect();

        // Only autocomplete the first word (command)
        if words.is_empty() || (!current_line.ends_with(' ') && words.len() == 1) {
            let partial = if words.is_empty() { "" } else { words[0] };
            let matches = CommandParser::autocomplete(partial);

            match matches.len() {
                0 => {
                    // No matches - do nothing
                }
                1 => {
                    // Single match - complete it
                    let completion = matches[0];
                    let partial_len = partial.len();

                    // Clear current partial command
                    for _ in 0..partial_len {
                        if self.cursor_pos > 0 {
                            self.line_buffer.pop();
                            self.cursor_pos -= 1;
                            self.write_str("\x08 \x08")?;
                        }
                    }
                    // Write the completion
                    for ch in completion.chars() {
                        if self.line_buffer.len() < CLI_BUFFER_SIZE - 1 {
                            self.line_buffer.push(ch);
                            self.cursor_pos += 1;
                            self.uart_tx
                                .write(&[ch as u8])
                                .map_err(|_| CliError::UartError)?;
                        }
                    }
                    // Add a space after completion
                    if self.line_buffer.len() < CLI_BUFFER_SIZE - 1 {
                        self.line_buffer.push(' ');
                        self.cursor_pos += 1;
                        self.uart_tx.write(b" ").map_err(|_| CliError::UartError)?;
                    }
                }
                _ => {
                    // Multiple matches - show them
                    self.write_str("\r\n")?;
                    for (i, cmd) in matches.iter().enumerate() {
                        if i > 0 {
                            self.write_str("  ")?;
                        }
                        self.write_str(cmd)?;
                    }
                    self.write_str("\r\n")?;
                    // Redraw prompt and current line
                    self.print_prompt()?;
                    self.write_str(&current_line)?;
                }
            }
        }
        Ok(())
    }

    pub fn show_help(&mut self) -> Result<(), CliError> {
        self.write_line("Available commands:")?;
        self.write_line("  help        - Show this help")?;
        self.write_line("  version     - Show firmware version")?;
        self.write_line("  status      - Show system status")?;
        self.write_line("  uptime      - Show system uptime")?;
        self.write_line("  clear       - Clear terminal")?;
        self.write_line("  reset       - Reset system")?;
        self.write_line("  echo <text> - Echo text back")?;
        self.write_line("  mtu_start [dur] - Start MTU operation (default 30s)")?;
        self.write_line("  mtu_stop    - Stop MTU operation")?;
        self.write_line("  mtu_status  - Show MTU status")?;
        self.write_line("  mtu_baud <rate> - Set MTU baud rate (1-115200, default 1200)")?;
        self.write_line("  mtu_format <fmt> - Set UART format (7E1, 7E2, 8N1, 8E1, 7O1, 8N2)")?;
        self.write_line("  mtu_reset   - Reset MTU statistics")?;
        self.write_line("  wifi_connect [ssid] [password] - Connect to WiFi (no args = default)")?;
        self.write_line("  wifi_reconnect - Quick reconnect to default WiFi")?;
        self.write_line("  wifi_status - Show WiFi connection status")?;
        self.write_line("  wifi_scan   - Scan for available WiFi networks")?;
        self.write_line("  mqtt_connect <broker_url> - Connect to MQTT broker")?;
        self.write_line("  mqtt_status - Show MQTT connection status")?;
        self.write_line("  mqtt_publish <topic> <message> - Publish MQTT message")?;
        self.write_line("")?;
        self.write_line("Use TAB to autocomplete commands")?;
        self.write_line("Use UP/DOWN arrows to navigate command history")?;
        self.write_line("Use LEFT/RIGHT arrows to move cursor and edit")?;
        Ok(())
    }

    pub fn show_meter_help(&mut self) -> Result<(), CliError> {
        self.write_line("Available commands:")?;
        self.write_line("  help        - Show this help")?;
        self.write_line("  version     - Show firmware version")?;
        self.write_line("  status      - Show meter status and statistics")?;
        self.write_line("  uptime      - Show system uptime")?;
        self.write_line("  clear       - Clear terminal")?;
        self.write_line("  reset       - Reset system")?;
        self.write_line("  enable      - Enable meter response to clock signals")?;
        self.write_line("  disable     - Disable meter response")?;
        self.write_line("  type <sensus|neptune> - Set meter type (7E1 or 7E2)")?;
        self.write_line("  format <fmt> - Set UART format (7E1, 7E2, 8N1, 8E1, 7O1, 8N2)")?;
        self.write_line("  message <text> - Set response message (\\r added automatically)")?;
        self.write_line("")?;
        self.write_line("Use TAB to autocomplete commands")?;
        self.write_line("Use UP/DOWN arrows to navigate command history")?;
        self.write_line("Use LEFT/RIGHT arrows to move cursor and edit")?;
        Ok(())
    }

    fn handle_history_up(&mut self) -> Result<(), CliError> {
        if self.command_history.is_empty() {
            return Ok(());
        }

        let new_index = match self.history_index {
            None => self.command_history.len() - 1,
            Some(current) => {
                if current > 0 {
                    current - 1
                } else {
                    return Ok(()); // Already at oldest command
                }
            }
        };

        self.history_index = Some(new_index);
        self.replace_current_line(&self.command_history[new_index].clone())
    }

    fn handle_history_down(&mut self) -> Result<(), CliError> {
        let new_index = match self.history_index {
            None => return Ok(()), // Not in history mode
            Some(current) => {
                if current < self.command_history.len() - 1 {
                    Some(current + 1)
                } else {
                    None // Back to empty line
                }
            }
        };

        self.history_index = new_index;

        match new_index {
            Some(idx) => self.replace_current_line(&self.command_history[idx].clone()),
            None => {
                // Clear line - back to empty
                let empty_line = String::new();
                self.replace_current_line(&empty_line)
            }
        }
    }

    fn replace_current_line(&mut self, new_line: &str) -> Result<(), CliError> {
        // Clear current line
        for _ in 0..self.cursor_pos {
            self.write_str("\x08 \x08")?;
        }

        // Update buffer and cursor
        self.line_buffer.clear();
        self.line_buffer.push_str(new_line);
        self.cursor_pos = new_line.len();

        // Display new line
        self.write_str(new_line)
    }

    fn handle_cursor_right(&mut self) -> Result<(), CliError> {
        if self.cursor_pos < self.line_buffer.len() {
            self.cursor_pos += 1;
            // Send ANSI escape sequence to move cursor right
            self.write_str("\x1b[C")?;
        }
        Ok(())
    }

    fn handle_cursor_left(&mut self) -> Result<(), CliError> {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            // Send ANSI escape sequence to move cursor left
            self.write_str("\x1b[D")?;
        }
        Ok(())
    }

    fn insert_char_at_cursor(&mut self, ch: char) -> Result<(), CliError> {
        if self.cursor_pos == self.line_buffer.len() {
            // Simple case: inserting at end
            self.line_buffer.push(ch);
            self.cursor_pos += 1;
            // Echo the character
            self.uart_tx
                .write(&[ch as u8])
                .map_err(|_| CliError::UartError)?;
        } else {
            // Complex case: inserting in middle - need to rebuild string
            self.line_buffer.insert(self.cursor_pos, ch);
            self.cursor_pos += 1;

            // Redraw from cursor position to end of line
            self.redraw_line_from_cursor()?;
        }
        Ok(())
    }

    fn redraw_line_from_cursor(&mut self) -> Result<(), CliError> {
        // Save current cursor position
        let saved_cursor = self.cursor_pos;

        // Get the part of the line from current cursor to end
        let chars_to_redraw: String = self.line_buffer.chars().skip(saved_cursor - 1).collect();

        // Write the characters from cursor position onward
        self.write_str(&chars_to_redraw)?;

        // Move cursor back to correct position
        let chars_written = chars_to_redraw.len();
        if chars_written > 1 {
            // Move cursor back (chars_written - 1) positions
            for _ in 1..chars_written {
                self.write_str("\x1b[D")?;
            }
        }

        Ok(())
    }

    fn delete_char_before_cursor(&mut self) -> Result<(), CliError> {
        if self.cursor_pos == self.line_buffer.len() {
            // Simple case: deleting from end
            self.line_buffer.pop();
            self.cursor_pos -= 1;
            // Send backspace sequence: backspace + space + backspace
            self.write_str("\x08 \x08")?;
        } else {
            // Complex case: deleting from middle
            self.line_buffer.remove(self.cursor_pos - 1);
            self.cursor_pos -= 1;

            // Move cursor left, then redraw from current position to end
            self.write_str("\x1b[D")?; // Move cursor left
            self.redraw_line_from_cursor_with_clear()?;
        }
        Ok(())
    }

    fn redraw_line_from_cursor_with_clear(&mut self) -> Result<(), CliError> {
        // Save current cursor position
        let saved_cursor = self.cursor_pos;

        // Get the part of the line from current cursor to end
        let chars_to_redraw: String = self.line_buffer.chars().skip(saved_cursor).collect();

        // Write the characters from cursor position onward
        self.write_str(&chars_to_redraw)?;

        // Clear the extra character that was there before
        self.write_str(" ")?;

        // Move cursor back to correct position
        let total_chars_written = chars_to_redraw.len() + 1; // +1 for the space
        for _ in 0..total_chars_written {
            self.write_str("\x1b[D")?;
        }

        Ok(())
    }
}
