# ESP32 LED Flasher

A simple yet powerful LED flasher application for ESP32 with configurable pulse timing, WiFi connectivity, and MQTT remote control.

## Features

- **Configurable LED Pulse**: Set pulse duration (1-2000ms) and period (3-3600s)
- **Multiple Control Interfaces**:
  - Serial CLI over USB
  - MQTT remote control (optional, disabled by default)
- **WiFi Connectivity**: Maintains persistent WiFi connection
- **Optional MQTT**: Enable with `mqtt_enable` command for instant remote control
- **Default Behavior**: Starts with 500ms pulse every 5 seconds
- **Built-in LED**: Uses GPIO2 (standard built-in LED on most ESP32 boards)

## Hardware Requirements

- ESP32 development board (ESP32-DevKitC, ESP32-WROOM, or similar)
- USB cable for programming and power
- Built-in LED on GPIO2 (most boards have this)

## Getting Started

### Prerequisites

- Rust toolchain with ESP32 support (xtensa-esp32-espidf target)
- espup (for installing ESP Rust toolchain)
- espflash for flashing firmware
- USB-to-UART drivers for your ESP32 board

### Installation

```bash
# Install ESP Rust toolchain
cargo install espup
espup install

# Install flashing tools
cargo install espflash

# Configure WiFi credentials in src/main.rs
# Edit lines 72-73:
const WIFI_SSID: &str = "YourSSID";
const WIFI_PASSWORD: &str = "YourPassword";
```

### Building and Flashing

```bash
# Build and flash (debug)
make flash

# Build and flash (release - smaller, optimized)
make flash-release

# Monitor serial output
make monitor
```

## Usage

### Serial CLI Commands

Connect to the ESP32 via USB and use any serial terminal (115200 baud):

```bash
# LED Control
led_on                          # Turn LED solid on
led_off                         # Turn LED off
led_pulse <dur_ms> <period_ms>  # Set custom pulse timing
led_status                      # Show current LED configuration
led_blink <frequency_hz>        # Set blink rate (1-10 Hz)

# System Commands
help                            # Show all commands
version                         # Show firmware version
status                          # Show system status
uptime                          # Show system uptime
reset                           # Reset system

# WiFi Commands
wifi_status                     # Show WiFi connection status
wifi_connect [ssid] [password]  # Connect to WiFi
wifi_scan                       # Scan for networks

# MQTT Commands
mqtt_status                     # Show MQTT status
mqtt_publish <topic> <msg>      # Publish MQTT message
```

### MQTT Control

The LED flasher subscribes to MQTT control topics and publishes status updates.

**Note:** MQTT is disabled by default. Use the `mqtt_enable` command via serial CLI to connect.

**Control Topics:**
- `istorrs/led/control` - Shared topic (all devices)
- `istorrs/led/<chip-id>/control` - Device-specific topic

**Status Topic:**
- `istorrs/led/<chip-id>/status` - Device publishes status every 60s

**MQTT Broker:** The project is configured to use Flespi (mqtt.flespi.io) with token authentication. Update credentials in `src/main.rs` lines 77-81.

**Control Commands:**

Simple commands (plain text):
```bash
# Turn LED on/off (requires Flespi token)
mosquitto_pub -h mqtt.flespi.io -u "FlespiToken YOUR_TOKEN" \
  -t "istorrs/led/control" -m "on"
mosquitto_pub -h mqtt.flespi.io -u "FlespiToken YOUR_TOKEN" \
  -t "istorrs/led/control" -m "off"
```

JSON commands (custom pulse):
```bash
# Set pulse: 500ms ON, 5000ms period
mosquitto_pub -h mqtt.flespi.io -u "FlespiToken YOUR_TOKEN" \
  -t "istorrs/led/control" -m '{"duration_ms":500,"period_ms":5000}'

# Set pulse: 100ms ON, 10000ms period (10s)
mosquitto_pub -h mqtt.flespi.io -u "FlespiToken YOUR_TOKEN" \
  -t "istorrs/led/control" -m '{"duration_ms":100,"period_ms":10000}'
```

**Status Messages:**

The device publishes JSON status messages:
```json
{
  "state": "pulsing",
  "duration_ms": 500,
  "period_ms": 5000,
  "device_id": "aa:bb:cc:dd:ee:ff"
}
```

## Configuration

### Pulse Timing Constraints

- **Duration**: 1ms to 2000ms (2 seconds)
- **Period**: 3000ms (3 seconds) to 3600000ms (1 hour)
- **Constraint**: Duration must be less than period

### WiFi and MQTT Settings

Edit `src/main.rs` to configure:

```rust
// WiFi Configuration (lines 72-73)
const WIFI_SSID: &str = "Your_SSID";
const WIFI_PASSWORD: &str = "Your_Password";

// MQTT Configuration (lines 77-81)
const MQTT_BROKER: &str = "mqtt://mqtt.flespi.io:1883";
const MQTT_USERNAME: &str = "FlespiToken YOUR_TOKEN_HERE";
const MQTT_PASSWORD: &str = "";
const MQTT_CONTROL_TOPIC_SHARED: &str = "istorrs/led/control";
// Device-specific topics are auto-generated based on chip ID
```

## Architecture

### Design Philosophy

The application maintains a persistent WiFi connection and offers optional MQTT connectivity:
- **WiFi Always-On**: WiFi connection is established at startup and maintained
- **MQTT On-Demand**: MQTT is disabled by default to save resources; enable with `mqtt_enable` command
- **Lower latency**: Once enabled, instant response to MQTT commands
- **Flexible deployment**: Can run without MQTT for standalone operation or with MQTT for remote control

### Background Tasks

1. **LED Task**: Manages LED state and pulse timing (runs in led.rs)
2. **MQTT Handler**: Processes incoming control messages (when enabled)
3. **Status Publisher**: Publishes LED status every 60 seconds (when MQTT enabled)
4. **CLI Loop**: Handles serial commands

## Examples

### Example 1: Quick Blink

```bash
# Via serial CLI
led_pulse 100 3000

# Via MQTT (requires mqtt_enable first and Flespi token)
mosquitto_pub -h mqtt.flespi.io -u "FlespiToken YOUR_TOKEN" \
  -t "istorrs/led/control" -m '{"duration_ms":100,"period_ms":3000}'
```

### Example 2: Long Slow Pulse

```bash
# Via serial CLI
led_pulse 2000 60000

# Via MQTT (requires mqtt_enable first and Flespi token)
mosquitto_pub -h mqtt.flespi.io -u "FlespiToken YOUR_TOKEN" \
  -t "istorrs/led/control" -m '{"duration_ms":2000,"period_ms":60000}'
```

### Example 3: Heartbeat Pattern

```bash
# 500ms flash every 5 seconds (default)
led_pulse 500 5000
```

## Troubleshooting

### WiFi Connection Issues

1. Check WiFi credentials in `src/main.rs`
2. Use `wifi_status` command to check connection
3. Use `wifi_scan` to see available networks
4. Use `wifi_connect <ssid> <password>` to reconnect

### MQTT Not Working

1. Enable MQTT with `mqtt_enable` command first
2. Verify WiFi is connected first (`wifi_status`)
3. Check MQTT broker URL and credentials in `src/main.rs` (lines 77-81)
4. Use `mqtt_status` command to check connection
5. Test with mosquitto_sub:
   ```bash
   mosquitto_sub -h mqtt.flespi.io -u "FlespiToken YOUR_TOKEN" \
     -t "istorrs/led/#" -v
   ```

### LED Not Flashing

1. Check LED status with `led_status` command
2. Verify GPIO2 has built-in LED on your board
3. Try `led_blink 1` for simple 1 Hz blink test
4. Check serial output for errors

## Development

### Project Structure

```
esp32-led-flasher/
├── src/
│   ├── main.rs           # Main application entry
│   ├── led.rs            # LED manager with pulse control
│   ├── cli/              # Command-line interface
│   │   ├── commands.rs   # Command handlers
│   │   ├── parser.rs     # Command parser
│   │   ├── terminal.rs   # Terminal I/O
│   │   └── mod.rs        # CLI module exports
│   ├── wifi.rs           # WiFi manager
│   ├── mqtt.rs           # MQTT client
│   ├── network_config.rs # Network configuration structures
│   └── lib.rs            # Library exports
├── Cargo.toml            # Rust dependencies
├── Makefile              # Build targets
├── rust-toolchain.toml   # Rust toolchain configuration
└── README.md             # This file
```

### Building from Source

```bash
# Debug build (faster compilation, larger binary)
make build

# Release build (slower compilation, optimized binary)
make release

# Flash and monitor
make flash monitor
```

## License

MIT OR Apache-2.0

## Acknowledgments

- Built with [esp-idf-rs](https://github.com/esp-rs/esp-idf-rs)
- Uses [ESP-IDF](https://github.com/espressif/esp-idf) v5.2.3
- MQTT broker: [Flespi](https://flespi.com/) with token authentication
- Rust toolchain managed by [espup](https://github.com/esp-rs/espup)
