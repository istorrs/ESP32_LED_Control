# ESP32 Water Meter MTU/Meter System

Complete water meter testing system with MTU (Meter Transmission Unit) reader and Meter simulator for ESP32 using Rust and ESP-IDF.

## Overview

This project provides **two separate applications**:

1. **MTU App** (`mtu_app`) - Reads water meter data by generating clock signals and capturing serial responses
2. **Meter App** (`meter_app`) - Simulates a water meter responding to MTU clock signals with configurable messages

Both apps feature interactive serial CLI control over UART0 (115200 baud, USB-C connection).

## Features

### Common Features
- **Serial CLI**: Interactive command-line interface with history, line editing, TAB autocompletion
- **Background Thread Architecture**: Non-blocking operations with main CLI thread
- **GPIO Communication**: 1200 baud serial over GPIO4 (clock) and GPIO5 (data)

### MTU App Features
- Hardware timer ISR for precise 1200 baud clock generation
- Automatic idle line synchronization for reliable message capture
- Early exit on complete message reception
- Message validation with parity checking and statistics
- **Configurable UART formats**: Support for 7E1, 7E2, 8N1, 8E1, 7O1, 8N2
- **On-demand WiFi/MQTT**: Connects only when publishing data (50-76% power savings)
- **Per-device MQTT control**: Device-specific and broadcast control topics
- **Remote configuration**: Change baud rate, UART format, trigger reads via MQTT
- **Device identification**: Unique chip_id, WiFi MAC, and IP in every message

### Meter App Features
- GPIO interrupt-based clock detection (rising edge)
- **Configurable UART formats**: Pre-computed frame generation for 7E1, 7E2, 8N1, 8E1, 7O1, 8N2
- Wake-up threshold (10 pulses) before transmission
- Configurable meter types (Sensus 7E1, Neptune 7E2)
- Customizable response messages and UART format via CLI

## Hardware

- **MCU**: ESP32 (Xtensa dual-core)
- **Framework**: ESP-IDF (std Rust)
- **HAL**: esp-idf-hal

## GPIO Pin Assignments

### Both Apps (Common)
- **UART0 (USB-C)**: GPIO1 (TX), GPIO3 (RX) - 115200 baud CLI

### MTU App
- **Clock (GPIO4)**:
  - Mode: Push-pull output
  - Initial state: LOW (simulates no power to meter)
  - Function: Generates 1200 baud clock signal for meter

- **Data (GPIO5)**:
  - Mode: Floating input (no pull-up/down)
  - Function: Reads serial data from meter response
  - Note: Line driven by meter's output (idle HIGH)
  - For 5V meters: Connect via level shifter, pull-up on 5V side

### Meter App
- **Clock (GPIO4)**:
  - Mode: Floating input (no pull-up/down)
  - Interrupt: Rising edge
  - Function: Detects MTU clock pulses
  - Note: Line driven by MTU's output

- **Data (GPIO5)**:
  - Mode: Push-pull output
  - Initial state: HIGH (UART idle state)
  - Function: Sends serial data to MTU

### ESP32-to-ESP32 Testing Configuration

For testing with two ESP32 boards (no external components required):
```
MTU GPIO4 (output) ──→ Meter GPIO4 (input + interrupt)
MTU GPIO5 (input)  ←── Meter GPIO5 (output)
MTU GND            ──── Meter GND
```

**Notes**:
- No external pull-up/pull-down resistors needed
- Lines are driven by push-pull outputs
- Direct GPIO-to-GPIO connection works reliably
- Keep wire length short (<30cm) for clean signals at 1200 baud

### Connecting to Real Water Meters

**For 5V water meters**, you need a bidirectional level shifter:

```
ESP32 (3.3V) ──> Level Shifter ──> Water Meter (5V)
  GPIO4 (clock)    TXS0102/BSS138      Clock In
  GPIO5 (data)                         Data Out
  GND                                  GND
```

**⚠️ WARNING**: Never connect 5V signals directly to ESP32 GPIO - use a level shifter!

See [docs/HARDWARE_SETUP.md](docs/HARDWARE_SETUP.md) for:
- Level shifter wiring diagrams
- Pull-up resistor requirements
- Power supply considerations
- Troubleshooting guide
- Signal integrity recommendations

## WiFi/MQTT Configuration (MTU App)

The MTU app supports on-demand WiFi/MQTT connectivity for remote monitoring and control.

### Configuration

Edit `src/main.rs` WiFi and MQTT credentials:

```rust
// WiFi Configuration
const WIFI_SSID: &str = "YOUR_SSID";
const WIFI_PASSWORD: &str = "YOUR_PASSWORD";

// MQTT Configuration
const MQTT_BROKER: &str = "mqtt://test.mosquitto.org:1883";
const MQTT_PUBLISH_TOPIC: &str = "istorrs/mtu/data";
const MQTT_CONTROL_TOPIC_SHARED: &str = "istorrs/mtu/control";
```

### On-Demand Mode

WiFi/MQTT operates in **on-demand mode**:
1. Disconnected by default while idle
2. After MTU read: Connects WiFi → MQTT
3. Subscribes to control topics (receives configuration)
4. Publishes meter data with device identification
5. Waits 5s for queued downlink messages
6. Disconnects MQTT → WiFi

**Power savings**: 50-76% compared to always-on WiFi/MQTT

### MQTT Topics

Each device subscribes to TWO control topics:
- **Shared**: `istorrs/mtu/control` (broadcast commands)
- **Device-specific**: `istorrs/mtu/{chip_id}/control` (per-device config)

Publishes data to: `istorrs/mtu/data` (with chip_id in payload)

See [docs/mqtt-control.md](docs/mqtt-control.md) for complete MQTT documentation.

## Prerequisites

1. **Rust ESP toolchain**:
   ```bash
   # Install Rust
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

   # Install ESP Rust toolchain
   cargo install espup
   espup install
   source ~/export-esp.sh  # Add to your shell profile
   ```

2. **espflash**:
   ```bash
   cargo install espflash
   ```

## Building and Flashing

### Using Makefile (Recommended)

```bash
# MTU App
make build              # Build MTU (debug)
make flash              # Flash MTU (debug)
make flash-release      # Flash MTU (release)

# Meter App
make build-meter        # Build Meter (debug)
make flash-meter        # Flash Meter (debug)
make flash-meter-release # Flash Meter (release)

# Utilities
make monitor            # Serial monitor
make clean              # Clean build
make help               # Show all commands
```

### Using Cargo Directly

```bash
# MTU App
cargo build --bin mtu_app --release
cargo run --bin mtu_app --release

# Meter App
cargo build --bin meter_app --release
cargo run --bin meter_app --release
```

## CLI Commands

Once flashed, connect via USB-C and use a serial terminal (115200 baud).

### MTU App Commands

```
ESP32 CLI> help

Available commands:
  help             - Show this help
  version          - Show firmware version
  status           - Show system status
  uptime           - Show system uptime
  clear            - Clear terminal
  reset            - Reset system
  echo <text>      - Echo text back

  mtu_start [dur]  - Start MTU operation (default 30s)
  mtu_stop         - Stop MTU operation
  mtu_status       - Show MTU status and statistics
  mtu_baud <rate>  - Set MTU baud rate (1-115200, default 1200)
  mtu_format <fmt> - Set UART format (7E1, 7E2, 8N1, 8E1, 7O1, 8N2)
  mtu_reset        - Reset MTU statistics

  wifi_connect [ssid] [password] - Connect to WiFi
  wifi_reconnect   - Quick reconnect to default WiFi
  wifi_status      - Show WiFi connection status
  wifi_scan        - Scan for available WiFi networks
  mqtt_status      - Show MQTT connection status (on-demand mode)
```

### Meter App Commands

```
ESP32 CLI> help

Available commands:
  help             - Show this help
  version          - Show firmware version
  status           - Show meter status and statistics
  uptime           - Show system uptime
  clear            - Clear terminal
  reset            - Reset system

  enable           - Enable meter response to clock signals
  disable          - Disable meter response
  type <sensus|neptune> - Set meter type (7E1 or 7E2)
  format <fmt>     - Set UART format (7E1, 7E2, 8N1, 8E1, 7O1, 8N2)
  message <text>   - Set response message (\r added automatically)
```

### Example Usage

#### MTU App
```bash
# Start MTU operation for 30 seconds
ESP32 CLI> mtu_start

# Check status
ESP32 CLI> mtu_status
MTU Status:
  State: Stopped
  Baud rate: 1200 bps
  Pins: GPIO4 (clock), GPIO5 (data)
  Total cycles: 5091
  Statistics:
    Successful reads: 1
    Corrupted reads: 0
    Success rate: 100.0%
  Last message: V;RB00000200;IB61564400;A1000;Z3214;...

# Set different baud rate
ESP32 CLI> mtu_baud 2400

# Start 60 second operation
ESP32 CLI> mtu_start 60

# Check WiFi status
ESP32 CLI> wifi_status
WiFi Status: Disconnected

# After MTU read completes, WiFi connects automatically to publish data
# Then disconnects (on-demand mode)
```

### MQTT Remote Control

Send commands and configuration via MQTT (while ESP32 is listening):

```bash
# Set baud rate (persists across reconnects with retain flag)
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"baud_rate":1200}' -q 1 -r

# Set UART format (7E1, 7E2, 8N1, 8E1, 7O1, 8N2)
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"uart_format":"7E1"}' -q 1 -r

# Trigger MTU read remotely
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"command":"start","duration":60}' -q 1

# Stop MTU operation
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"command":"stop"}' -q 1

# Monitor meter data (with device identification)
mosquitto_sub -h test.mosquitto.org -t "istorrs/mtu/data"
```

See [docs/mqtt-control.md](docs/mqtt-control.md) for complete MQTT documentation including per-device topics.

```

#### Meter App
```bash
# Check meter status
ESP32 CLI> status
Meter Status:
  State: Enabled
  Type: Sensus
  Pins: GPIO4 (clock in), GPIO5 (data out)
  Message: 'V;RB00000200;IB61564400;...' (70 chars)
  Statistics:
    Clock pulses: 5091
    Bits transmitted: 560
    Messages sent: 1
    Currently transmitting: No

# Set custom message
ESP32 CLI> message TEST123

# Change meter type to Neptune (7E2)
ESP32 CLI> type neptune

# Disable response
ESP32 CLI> disable
```

## Architecture

### MTU App Architecture

#### Thread Model
- **Main Thread**: UART CLI loop processing user input
- **MTU Thread**: Background thread owning GPIO pins and hardware timer
  - Receives commands via `mpsc::channel`
  - Spawns per-operation UART framing task
  - Reusable timer ISR for unlimited operations
- **MQTT Thread**: Created on-demand for each publish cycle
  - Connection handler thread for event processing
  - Graceful shutdown prevents reconnect errors

#### Operation Flow
1. Power-up sequence (clock HIGH, 10ms delay)
2. Hardware timer generates 4-phase clock signal (4800 Hz for 1200 baud)
3. Timer ISR increments cycle counter and notifies GPIO task
4. GPIO task toggles clock pin and samples data line
5. UART framing task processes bit stream into 7E1 frames
6. Validates parity and stop bit, extracts ASCII characters
7. Exits early on carriage return (`\r`) or timeout
8. Clock pin set LOW to simulate no power to meter
9. **On-demand publish** (if WiFi configured):
   - Connect WiFi (~2-5s)
   - Create MQTT client and subscribe to control topics
   - Publish meter data with device identification (chip_id, wifi_mac, wifi_ip)
   - Wait 5s for queued downlink messages (baud rate config, start/stop commands)
   - Gracefully shutdown MQTT connection handler
   - Disconnect WiFi

#### Technical Details
- **Timer ISR → Task Pattern**: Hardware timer ISR for precise timing, FreeRTOS task for GPIO
- **Idle Line Sync**: Waits for 10 consecutive 1-bits before frame detection
- **Efficiency**: ~83-84% ISR notification → task handling efficiency
- **Early Exit**: Completes immediately upon receiving `\r`
- **Power Simulation**: Clock LOW at bootup and after operations

### Meter App Architecture

#### Thread Model
- **Main Thread**: UART CLI loop processing user input
- **Meter Thread**: Background thread with GPIO interrupt handler
  - Clock pin interrupt (rising edge) triggers ISR
  - ISR notifies task via FreeRTOS notification
  - Task outputs pre-computed bits on data line

#### Operation Flow
1. Clock pin idle (LOW from MTU)
2. Clock rising edge triggers GPIO interrupt (ISR)
3. ISR sends notification to meter task (minimal work)
4. Task increments pulse counter
5. After 10 pulses (wake-up threshold), builds response frames
6. On each subsequent clock pulse, outputs next bit on data line
7. Returns to idle after complete message transmitted

#### Technical Details
- **ISR → Notification Pattern**: Minimal ISR work, heavy lifting in task
- **Wake-up Threshold**: 10 pulses before transmission starts
- **Pre-computed Frames**: Message → ASCII → UART frames (7E1/7E2) → bit array
- **State Machine**: Idle → Wake-up → Transmitting → Complete
- **Frame Format**: Configurable 7E1 (Sensus) or 7E2 (Neptune)

### Message Format
- **Protocol**: 7E1 or 7E2 UART
  - 7E1: 7 data bits, even parity, 1 stop bit (Sensus)
  - 7E2: 7 data bits, even parity, 2 stop bits (Neptune)
- **Baud Rate**: 1200 bps (default)
- **Message**: ASCII text ending with `\r`
- **Example**: `V;RB00000200;IB61564400;A1000;Z3214;XT0746;MT0683;...`

## License

MIT OR Apache-2.0
