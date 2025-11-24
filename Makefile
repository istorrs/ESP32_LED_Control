# Makefile for ESP32 LED Flasher (ESP-IDF)

.PHONY: all build flash release flash-release monitor clean help

# Default target
all: build

# Build LED Flasher (debug)
build:
	@echo "🔧 Building ESP32 LED Flasher app (debug) with ESP-IDF..."
	cargo build --bin led_app

# Build LED Flasher (release)
release:
	@echo "🔧 Building ESP32 LED Flasher app (release) with ESP-IDF..."
	cargo build --bin led_app --release

# Flash LED Flasher (debug)
flash: build
	@echo "📱 Flashing ESP32 LED Flasher app (debug)..."
	cargo run --bin led_app

# Flash LED Flasher (release)
flash-release: release
	@echo "📱 Flashing ESP32 LED Flasher app (release)..."
	cargo run --bin led_app --release

# Monitor
monitor:
	@echo "🖥️  Opening serial monitor..."
	espflash serial-monitor

# Clean
clean:
	@echo "🧹 Cleaning build artifacts..."
	cargo clean

# Help
help:
	@echo "ESP32 LED Flasher (ESP-IDF) - Available Commands:"
	@echo ""
	@echo "Build & Flash:"
	@echo "  make build              - Build LED Flasher app (debug)"
	@echo "  make release            - Build LED Flasher app (release)"
	@echo "  make flash              - Flash LED Flasher app (debug)"
	@echo "  make flash-release      - Flash LED Flasher app (release)"
	@echo ""
	@echo "Utilities:"
	@echo "  make monitor            - Open serial monitor"
	@echo "  make clean              - Clean build artifacts"
	@echo "  make help               - Show this help"
	@echo ""
	@echo "Configuration:"
	@echo "  LED: GPIO2 (built-in LED on most ESP32 boards)"
	@echo "  Default pulse: 500ms ON / 5s period"
	@echo ""
	@echo "Control:"
	@echo "  Serial CLI - Connect via USB and use commands like 'led_pulse 500 5000'"
	@echo "  MQTT - Publish to istorrs/led/control or device-specific topic"
	@echo ""
	@echo "Note: ESP-IDF will be automatically downloaded and configured on first build"
