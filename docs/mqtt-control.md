# MQTT Control Messages

The ESP32 water meter MTU interface supports remote control and configuration via MQTT messages on the control topic.

## Topics

Each ESP32 subscribes to TWO control topics:

- **Shared Control Topic**: `istorrs/mtu/control` (broadcast commands to all devices)
- **Device Control Topic**: `istorrs/mtu/{chip_id}/control` (commands for specific device)
- **Data Topic**: `istorrs/mtu/data` (published by all devices with chip_id in payload)

Example for device with chip_id `24:0a:c4:12:34:56`:
- Subscribes to: `istorrs/mtu/control` AND `istorrs/mtu/24:0a:c4:12:34:56/control`
- Publishes to: `istorrs/mtu/data`

## Data Payload Format

Each meter reading published to `istorrs/mtu/data` includes device identification:

```json
{
  "chip_id": "24:0a:c4:12:34:56",
  "wifi_mac": "24:0a:c4:12:34:57",
  "wifi_ip": "192.168.1.119",
  "message": "V;RB00000200;IB61564400;A1000;Z3214;XT0746;MT0683;RR00000000;GX000000;GN000000",
  "baud_rate": 1200,
  "uart_format": "7E1",
  "cycles": 15,
  "successful": 2,
  "corrupted": 0,
  "count": 5
}
```

**Device Identification Fields**:
- `chip_id` - ESP32 base MAC address from eFuse (unique identifier, persists across reboots)
- `wifi_mac` - WiFi station MAC address (may differ from chip_id)
- `wifi_ip` - Current IP address assigned by DHCP

**Meter Data Fields**:
- `message` - Raw meter response string
- `baud_rate` - Current MTU baud rate setting (1-115200 bps)
- `uart_format` - Current UART frame format (7E1, 7E2, 8N1, 8E1, 7O1, 8N2)
- `cycles` - Total clock cycles sent
- `successful` - Number of successful reads
- `corrupted` - Number of corrupted reads (frame errors)
- `count` - Sequential message counter

## Message Formats

### JSON Format (Recommended)

JSON messages provide a structured way to send configuration and commands.

#### Set Baud Rate

Configure the MTU baud rate (persists across ESP32 reconnects when using `retain`).

**Broadcast to all devices**:
```bash
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"baud_rate":1200}' -q 1 -r
```

**Send to specific device** (recommended for multi-device setups):
```bash
# Replace chip_id with your device's chip_id
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/24:0a:c4:12:34:56/control" \
  -m '{"baud_rate":1200}' -q 1 -r
```

**Supported baud rates**: 1-115200 bps (typical water meters: 300, 600, 1200, 9600)

**Important**:
- Use **QoS 1** (`-q 1`) for reliable delivery
- Use **retain** (`-r`) so the configuration persists and is delivered to ESP32 on every connect
- Baud rate can only be changed when MTU is stopped
- Device-specific topics prevent accidentally changing all devices

#### Set UART Format

Configure the MTU UART frame format (persists across ESP32 reconnects when using `retain`).

**Broadcast to all devices**:
```bash
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"uart_format":"7E1"}' -q 1 -r
```

**Send to specific device** (recommended for multi-device setups):
```bash
# Replace chip_id with your device's chip_id
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/24:0a:c4:12:34:56/control" \
  -m '{"uart_format":"8N1"}' -q 1 -r
```

**Supported UART formats**:
- `7E1` - 7 data bits, even parity, 1 stop bit (Sensus meters - default)
- `7E2` - 7 data bits, even parity, 2 stop bits (Neptune meters)
- `8N1` - 8 data bits, no parity, 1 stop bit (generic)
- `8E1` - 8 data bits, even parity, 1 stop bit
- `7O1` - 7 data bits, odd parity, 1 stop bit
- `8N2` - 8 data bits, no parity, 2 stop bits

**Important**:
- Use **QoS 1** (`-q 1`) for reliable delivery
- Use **retain** (`-r`) so the configuration persists
- UART format can only be changed when MTU is stopped
- Must match your water meter's UART configuration

#### Start MTU with Duration

Start the MTU for a specific duration.

**Broadcast** (all devices start):
```bash
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"command":"start","duration":60}' -q 1
```

**Specific device**:
```bash
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/24:0a:c4:12:34:56/control" \
  -m '{"command":"start","duration":60}' -q 1
```

Duration is in seconds (default: 30s if not specified).

#### Stop MTU

Stop the currently running MTU operation.

**Broadcast** (all devices stop):
```bash
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"command":"stop"}' -q 1
```

**Specific device**:
```bash
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/24:0a:c4:12:34:56/control" \
  -m '{"command":"stop"}' -q 1
```

### Plain Text Format (Legacy)

For backwards compatibility, plain text commands are still supported:

```bash
# Start with default 30s duration
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m "start" -q 1

# Start with custom duration
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m "start 60" -q 1

# Stop
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m "stop" -q 1
```

## QoS Recommendations

### QoS 0 (At most once)
- Fire-and-forget delivery
- Message may be lost if network issues occur
- ⚠️ **Not recommended for important configuration**

### QoS 1 (At least once) ✅ **Recommended**
- Guaranteed delivery with acknowledgment
- Message will be retried if not acknowledged
- ✅ **Use for all control messages and configuration**

### QoS 2 (Exactly once)
- Highest overhead, rarely needed
- Not necessary for this application

## Retained Messages

Use the `-r` (retain) flag for **configuration messages** like baud rate:

```bash
# With retain - persists on broker
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"baud_rate":1200}' -q 1 -r
```

**Benefits**:
- Broker stores the message and delivers it to new subscribers
- ESP32 receives configuration on every connect (perfect for on-demand mode)
- Configuration survives ESP32 reboots

**Don't use retain for**:
- One-time commands like "start" or "stop"
- These should be delivered only once, not on every connect

## On-Demand Mode Behavior

In on-demand WiFi/MQTT mode, the ESP32:

1. **Disconnected by default** - No WiFi/MQTT connection while idle
2. **After MTU read** - Connects WiFi → Connects MQTT
3. **Subscribes to control topic** - Receives any retained messages
4. **Applies configuration** - e.g., changes baud rate if message received
5. **Publishes data** - Sends MTU reading to data topic
6. **Waits 5 seconds** - Listens for any queued downlink messages
7. **Disconnects** - Drops MQTT → Drops WiFi

This means:
- **Retained messages** are received on every publish cycle
- **Non-retained messages** sent while offline are only delivered if QoS 1+ (queued by broker)
- Configuration changes apply before the **next** MTU read

## Topic Strategy

### Shared vs Device-Specific Topics

**Shared Topic** (`istorrs/mtu/control`):
- ✅ Use for broadcast commands (all devices respond)
- ✅ Good for emergency stop, synchronized operations
- ⚠️ Retained messages affect ALL devices on connect

**Device-Specific Topic** (`istorrs/mtu/{chip_id}/control`):
- ✅ Use for per-device configuration (baud rate, etc.)
- ✅ Retained messages only affect the specific device
- ✅ Prevents accidental configuration of wrong device
- ✅ Required for multi-device deployments

### Multi-Device Strategy

If you have multiple ESP32s reading different meters:

1. **Get each device's chip_id** from data messages
2. **Set device-specific baud rates**:
   ```bash
   # Device A - Sensus meter at 1200 baud
   mosquitto_pub -h test.mosquitto.org \
     -t "istorrs/mtu/24:0a:c4:aa:bb:cc/control" \
     -m '{"baud_rate":1200}' -q 1 -r

   # Device B - Neptune meter at 9600 baud
   mosquitto_pub -h test.mosquitto.org \
     -t "istorrs/mtu/24:0a:c4:dd:ee:ff/control" \
     -m '{"baud_rate":9600}' -q 1 -r
   ```

3. **Trigger reads individually or all at once**:
   ```bash
   # Start all devices
   mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
     -m '{"command":"start","duration":30}' -q 1

   # Or start specific device
   mosquitto_pub -h test.mosquitto.org \
     -t "istorrs/mtu/24:0a:c4:aa:bb:cc/control" \
     -m '{"command":"start","duration":30}' -q 1
   ```

4. **Filter data by device**:
   ```bash
   mosquitto_sub -h test.mosquitto.org -t "istorrs/mtu/data" | \
     jq 'select(.chip_id == "24:0a:c4:aa:bb:cc")'
   ```

## Example Workflow

### 1. Set baud rate for your meter

```bash
# Set to 1200 bps (typical for Sensus meters)
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"baud_rate":1200}' -q 1 -r
```

This message is now stored by the broker and will be delivered every time the ESP32 connects.

### 2. Trigger a meter read

You can trigger reads in two ways:

**Option A**: Via MQTT (while ESP32 is connected)
```bash
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"command":"start","duration":30}' -q 1
```

**Option B**: Via serial CLI
```
ESP32 CLI> mtu_start 30
```

### 3. ESP32 publishes data

After completing the read, ESP32:
- Connects to WiFi/MQTT
- Receives baud rate configuration (1200 bps)
- Publishes meter data to `istorrs/mtu/data`
- Disconnects

### 4. Change baud rate

If you need to change the baud rate:

```bash
# Update retained message
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/control" \
  -m '{"baud_rate":9600}' -q 1 -r
```

The new rate will be applied on the next connection (next meter read).

## Monitoring Messages

### Subscribe to Control Messages

See all control commands being sent:

```bash
mosquitto_sub -h test.mosquitto.org -t "istorrs/mtu/control" -v
```

### Subscribe to Meter Data

See all meter readings with device identification:

```bash
mosquitto_sub -h test.mosquitto.org -t "istorrs/mtu/data" -v
```

### Filter by Device

If you have multiple ESP32 devices, filter by chip ID using `jq`:

```bash
# Subscribe and filter for specific device
mosquitto_sub -h test.mosquitto.org -t "istorrs/mtu/data" | \
  jq 'select(.chip_id == "24:0a:c4:12:34:56")'

# Extract just the meter message
mosquitto_sub -h test.mosquitto.org -t "istorrs/mtu/data" | \
  jq -r '.message'

# Show device summary
mosquitto_sub -h test.mosquitto.org -t "istorrs/mtu/data" | \
  jq '{chip_id, ip: .wifi_ip, message: .message, baud: .baud_rate}'
```

### Send Control to Specific Device

Each device subscribes to its own device-specific topic:

```bash
# Get chip_id from data payload, then send device-specific command
mosquitto_pub -h test.mosquitto.org -t "istorrs/mtu/24:0a:c4:12:34:56/control" \
  -m '{"baud_rate":1200}' -q 1 -r
```

**Best practices**:
- Use device-specific topics when you have multiple ESP32s
- Use shared topic only for broadcast commands (restart all, etc.)
- Retained messages on device-specific topics won't affect other devices

## Troubleshooting

**Baud rate not changing?**
- Check that MTU is stopped (can't change baud rate while running)
- Verify message uses QoS 1 and retain flag
- Check ESP32 logs for "MQTT: Setting baud rate to X bps"

**Commands not received?**
- Verify broker is reachable: `ping test.mosquitto.org`
- Check QoS level (use QoS 1)
- For on-demand mode: commands are only received during the 5s window after publishing data
- Use retained messages for persistent configuration

**JSON parsing errors?**
- Verify JSON is valid: `echo '{"baud_rate":1200}' | jq .`
- Check quotes are properly escaped in shell
- ESP32 logs will show "Unknown control command" if JSON is invalid
