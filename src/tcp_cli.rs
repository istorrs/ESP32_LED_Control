//! TCP CLI Server — exposes the same interactive CLI as the UART serial port
//! over a raw TCP socket (Telnet-style).
//!
//! # Usage (from any machine on the same LAN)
//! ```bash
//! nc <esp32-ip> 2323
//! telnet <esp32-ip> 2323
//! socat -,rawer TCP:<esp32-ip>:2323   # full terminal experience with arrow keys
//! ```
//!
//! One client is served at a time.  When a new connection arrives the previous
//! one is replaced (the listener is in a loop; the old thread eventually exits
//! when it detects the stream is closed).
//!
//! No new Cargo dependencies are required — `std::net` TCP sockets are provided
//! by the ESP-IDF POSIX compatibility layer already linked into this project.

use crate::cli::{CliCommand, CliError, CommandHandler, CommandParser, CLI_BUFFER_SIZE};
use crate::led::LedManager;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

/// Telnet option bytes — sent by some clients, we respond "will not".
const IAC: u8 = 0xFF;
const DONT: u8 = 0xFE;
const DO: u8 = 0xFD;
const WONT: u8 = 0xFC;
const WILL: u8 = 0xFB;

pub const TCP_CLI_PORT: u16 = 2323;
const HISTORY_SIZE: usize = 10;

/// Start the TCP CLI server in a background thread.
///
/// Requires WiFi to be up before calling.  The `led` handle is cloned into
/// each connection handler thread so commands have access to the LED manager.
pub fn start(led: Arc<LedManager>) -> anyhow::Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", TCP_CLI_PORT))?;
    log::info!("🌐 TCP CLI server listening on port {}", TCP_CLI_PORT);

    std::thread::Builder::new()
        .stack_size(8192)
        .name("tcp_cli_listener".to_string())
        .spawn(move || {
            log::info!("📟 TCP CLI listener thread started");
            for stream_result in listener.incoming() {
                match stream_result {
                    Ok(stream) => {
                        let led_clone = led.clone();
                        if let Err(e) = handle_connection(stream, led_clone) {
                            log::warn!("TCP CLI connection error: {:?}", e);
                        }
                    }
                    Err(e) => {
                        log::warn!("TCP CLI accept error: {:?}", e);
                        // Brief pause before retrying
                        std::thread::sleep(Duration::from_millis(500));
                    }
                }
            }
            log::info!("📟 TCP CLI listener thread exiting");
        })
        .map_err(|e| anyhow::anyhow!("Failed to spawn TCP CLI listener thread: {}", e))?;

    Ok(())
}

/// Drive one connected client until it disconnects.
///
/// This runs synchronously in the listener thread (blocking for the duration
/// of the connection).  A single-client-at-a-time model keeps things simple
/// and avoids the need for a shared-mutable command handler.
fn handle_connection(stream: TcpStream, led: Arc<LedManager>) -> anyhow::Result<()> {
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_else(|_| "?".to_string());
    log::info!("📟 TCP CLI: new connection from {}", peer);

    // Read timeout — short enough for a snappy interactive loop without busywaiting
    stream.set_read_timeout(Some(Duration::from_millis(10)))?;

    let mut term = TcpTerminal::new(stream);
    let mut handler = CommandHandler::new().with_led(led);

    // Welcome banner (mirrors main.rs)
    term.write_line("\r\n")?;
    term.write_line("╔═══════════════════════════════════════════════════════╗")?;
    term.write_line("║       ESP32 LED Flasher — WiFi CLI (TCP port 2323)   ║")?;
    term.write_line("╚═══════════════════════════════════════════════════════╝")?;
    term.write_line("  Type 'help' for available commands")?;
    term.write_line("  (Arrow keys, Tab-complete and history all work)")?;
    term.write_line("")?;
    term.print_prompt()?;

    loop {
        match term.read_char() {
            Ok(Some(ch)) => {
                match term.handle_char(ch) {
                    Ok(Some(command_str)) => {
                        let command = CommandParser::parse_command(&command_str);

                        // Handle disconnect before passing to the generic command handler
                        if matches!(command, CliCommand::Disconnect) {
                            let _ = term.write_line("Goodbye!");
                            log::info!("TCP CLI: client {} requested disconnect", peer);
                            break;
                        }

                        match handler.execute_command(command) {
                            Ok(response) => {
                                if !response.is_empty() {
                                    if let Err(e) = term.write_line(&response) {
                                        log::info!("TCP CLI: client {} disconnected (write): {:?}", peer, e);
                                        break;
                                    }
                                }
                                if let Err(e) = term.print_prompt() {
                                    log::info!("TCP CLI: client {} disconnected (prompt): {:?}", peer, e);
                                    break;
                                }
                            }
                            Err(e) => {
                                let _ = term.write_line(&format!("Error: {}", e));
                                let _ = term.print_prompt();
                            }
                        }
                    }
                    Ok(None) => {
                        // Incomplete line, keep reading
                    }
                    Err(CliError::UartError) => {
                        log::info!("TCP CLI: client {} disconnected (read error)", peer);
                        break;
                    }
                    Err(_) => {}
                }
            }
            Ok(None) => {
                // No data yet — tight but yielding loop
            }
            Err(CliError::UartError) => {
                log::info!("TCP CLI: client {} disconnected", peer);
                break;
            }
            Err(_) => {}
        }
    }

    log::info!("📟 TCP CLI: connection from {} closed", peer);
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// TcpTerminal — mirrors Terminal<'d> but backed by TcpStream instead of UART
// ────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum EscapeState {
    Normal,
    Escape,
    Csi,
    /// Absorbing a telnet IAC option sub-sequence
    Iac(u8), // stores the verb byte (WILL/WONT/DO/DONT)
}

struct TcpTerminal {
    stream: TcpStream,
    line_buffer: String,
    cursor_pos: usize,
    command_history: Vec<String>,
    history_index: Option<usize>,
    escape_state: EscapeState,
}

impl TcpTerminal {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            line_buffer: String::new(),
            cursor_pos: 0,
            command_history: Vec::new(),
            history_index: None,
            escape_state: EscapeState::Normal,
        }
    }

    // ── I/O primitives ──────────────────────────────────────────────────────

    fn write_bytes(&mut self, data: &[u8]) -> Result<(), CliError> {
        self.stream.write_all(data).map_err(|_| CliError::UartError)
    }

    fn write_str(&mut self, s: &str) -> Result<(), CliError> {
        self.write_bytes(s.as_bytes())
    }

    fn write_line(&mut self, s: &str) -> Result<(), CliError> {
        self.write_str(s)?;
        self.write_str("\r\n")
    }

    fn print_prompt(&mut self) -> Result<(), CliError> {
        self.write_str("ESP32 CLI> ")
    }

    /// Non-blocking read of a single byte.  Returns `Ok(None)` when no data
    /// is available (EAGAIN / WouldBlock from the 10 ms read timeout).
    fn read_char(&mut self) -> Result<Option<u8>, CliError> {
        let mut buf = [0u8; 1];
        match self.stream.read(&mut buf) {
            Ok(1) => Ok(Some(buf[0])),
            Ok(0) => Err(CliError::UartError),  // EOF — client disconnected
            Ok(_) => Ok(None),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
                       || e.kind() == std::io::ErrorKind::TimedOut => Ok(None),
            Err(_) => Err(CliError::UartError),
        }
    }

    // ── Line editor ─────────────────────────────────────────────────────────

    fn handle_char(&mut self, ch: u8) -> Result<Option<String>, CliError> {
        match self.escape_state {
            EscapeState::Normal => match ch {
                b'\r' | b'\n' => {
                    self.write_str("\r\n")?;
                    let command = self.line_buffer.clone();
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
                    self.escape_state = EscapeState::Escape;
                    Ok(None)
                }
                b'\x08' | b'\x7f' => {
                    if !self.line_buffer.is_empty() && self.cursor_pos > 0 {
                        self.delete_char_before_cursor()?;
                    }
                    Ok(None)
                }
                b'\t' => {
                    self.handle_tab_completion()?;
                    Ok(None)
                }
                IAC => {
                    // Telnet IAC — next byte will be the verb
                    self.escape_state = EscapeState::Iac(0);
                    Ok(None)
                }
                0x20..=0x7E => {
                    if self.line_buffer.len() < CLI_BUFFER_SIZE - 1 {
                        self.insert_char_at_cursor(ch as char)?;
                    }
                    Ok(None)
                }
                _ => Ok(None),
            },

            EscapeState::Escape => {
                if ch == b'[' {
                    self.escape_state = EscapeState::Csi;
                } else {
                    self.escape_state = EscapeState::Normal;
                }
                Ok(None)
            }

            EscapeState::Csi => {
                self.escape_state = EscapeState::Normal;
                match ch {
                    b'A' => {
                        self.handle_history_up()?;
                    }
                    b'B' => {
                        self.handle_history_down()?;
                    }
                    b'C' => {
                        self.handle_cursor_right()?;
                    }
                    b'D' => {
                        self.handle_cursor_left()?;
                    }
                    _ => {}
                }
                Ok(None)
            }

            EscapeState::Iac(verb) => {
                if verb == 0 {
                    // We just received IAC; ch is the verb
                    match ch {
                        WILL | WONT | DO | DONT => {
                            self.escape_state = EscapeState::Iac(ch);
                        }
                        _ => {
                            // Single-byte IAC command (e.g. IAC AYT) — ignore
                            self.escape_state = EscapeState::Normal;
                        }
                    }
                } else {
                    // ch is the option byte — send a polite refusal and return to normal
                    let response_verb = if verb == WILL { DONT } else { WONT };
                    let _ = self.write_bytes(&[IAC, response_verb, ch]);
                    self.escape_state = EscapeState::Normal;
                }
                Ok(None)
            }
        }
    }

    // ── Tab completion ───────────────────────────────────────────────────────

    fn handle_tab_completion(&mut self) -> Result<(), CliError> {
        let current_line = self.line_buffer.clone();
        let words: Vec<&str> = current_line.split_whitespace().collect();
        if words.is_empty() || (!current_line.ends_with(' ') && words.len() == 1) {
            let partial = if words.is_empty() { "" } else { words[0] };
            let matches = CommandParser::autocomplete(partial);
            match matches.len() {
                0 => {}
                1 => {
                    let completion = matches[0];
                    for _ in 0..partial.len() {
                        if self.cursor_pos > 0 {
                            self.line_buffer.pop();
                            self.cursor_pos -= 1;
                            self.write_str("\x08 \x08")?;
                        }
                    }
                    for ch in completion.chars() {
                        if self.line_buffer.len() < CLI_BUFFER_SIZE - 1 {
                            self.line_buffer.push(ch);
                            self.cursor_pos += 1;
                            self.write_bytes(&[ch as u8])?;
                        }
                    }
                    if self.line_buffer.len() < CLI_BUFFER_SIZE - 1 {
                        self.line_buffer.push(' ');
                        self.cursor_pos += 1;
                        self.write_bytes(b" ")?;
                    }
                }
                _ => {
                    self.write_str("\r\n")?;
                    for (i, cmd) in matches.iter().enumerate() {
                        if i > 0 {
                            self.write_str("  ")?;
                        }
                        self.write_str(cmd)?;
                    }
                    self.write_str("\r\n")?;
                    self.print_prompt()?;
                    self.write_str(&current_line)?;
                }
            }
        }
        Ok(())
    }

    // ── History navigation ───────────────────────────────────────────────────

    fn handle_history_up(&mut self) -> Result<(), CliError> {
        if self.command_history.is_empty() {
            return Ok(());
        }
        let new_index = match self.history_index {
            None => self.command_history.len() - 1,
            Some(0) => return Ok(()),
            Some(current) => current - 1,
        };
        self.history_index = Some(new_index);
        self.replace_current_line(&self.command_history[new_index].clone())
    }

    fn handle_history_down(&mut self) -> Result<(), CliError> {
        let new_index = match self.history_index {
            None => return Ok(()),
            Some(current) if current < self.command_history.len() - 1 => Some(current + 1),
            _ => None,
        };
        self.history_index = new_index;
        let line = match new_index {
            Some(idx) => self.command_history[idx].clone(),
            None => String::new(),
        };
        self.replace_current_line(&line)
    }

    fn replace_current_line(&mut self, new_line: &str) -> Result<(), CliError> {
        for _ in 0..self.cursor_pos {
            self.write_str("\x08 \x08")?;
        }
        self.line_buffer.clear();
        self.line_buffer.push_str(new_line);
        self.cursor_pos = new_line.len();
        self.write_str(new_line)
    }

    // ── Cursor movement ──────────────────────────────────────────────────────

    fn handle_cursor_right(&mut self) -> Result<(), CliError> {
        if self.cursor_pos < self.line_buffer.len() {
            self.cursor_pos += 1;
            self.write_str("\x1b[C")?;
        }
        Ok(())
    }

    fn handle_cursor_left(&mut self) -> Result<(), CliError> {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            self.write_str("\x1b[D")?;
        }
        Ok(())
    }

    // ── Character insertion / deletion ───────────────────────────────────────

    fn insert_char_at_cursor(&mut self, ch: char) -> Result<(), CliError> {
        if self.cursor_pos == self.line_buffer.len() {
            self.line_buffer.push(ch);
            self.cursor_pos += 1;
            self.write_bytes(&[ch as u8])?;
        } else {
            self.line_buffer.insert(self.cursor_pos, ch);
            self.cursor_pos += 1;
            self.redraw_line_from_cursor()?;
        }
        Ok(())
    }

    fn delete_char_before_cursor(&mut self) -> Result<(), CliError> {
        if self.cursor_pos == self.line_buffer.len() {
            self.line_buffer.pop();
            self.cursor_pos -= 1;
            self.write_str("\x08 \x08")?;
        } else {
            self.line_buffer.remove(self.cursor_pos - 1);
            self.cursor_pos -= 1;
            self.write_str("\x1b[D")?;
            self.redraw_line_from_cursor_with_clear()?;
        }
        Ok(())
    }

    fn redraw_line_from_cursor(&mut self) -> Result<(), CliError> {
        let saved = self.cursor_pos;
        let tail: String = self.line_buffer.chars().skip(saved - 1).collect();
        self.write_str(&tail)?;
        let back = tail.len().saturating_sub(1);
        for _ in 0..back {
            self.write_str("\x1b[D")?;
        }
        Ok(())
    }

    fn redraw_line_from_cursor_with_clear(&mut self) -> Result<(), CliError> {
        let saved = self.cursor_pos;
        let tail: String = self.line_buffer.chars().skip(saved).collect();
        self.write_str(&tail)?;
        self.write_str(" ")?; // erase old last char
        let back = tail.len() + 1;
        for _ in 0..back {
            self.write_str("\x1b[D")?;
        }
        Ok(())
    }
}
