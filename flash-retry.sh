#!/bin/bash
# Retry flashing script for unreliable USB connections
# Usage: ./flash-retry.sh [release]

MAX_RETRIES=5
RETRY_COUNT=0

if [ "$1" == "release" ]; then
    BUILD_CMD="cargo build --bin led_app --release"
    FLASH_CMD="cargo run --bin led_app --release"
    echo "🔧 Building release version..."
else
    BUILD_CMD="cargo build --bin led_app"
    FLASH_CMD="cargo run --bin led_app"
    echo "🔧 Building debug version..."
fi

# Build once
$BUILD_CMD
if [ $? -ne 0 ]; then
    echo "❌ Build failed"
    exit 1
fi

echo "✅ Build successful"
echo ""

# Try flashing with retries
while [ $RETRY_COUNT -lt $MAX_RETRIES ]; do
    echo "📱 Flash attempt $((RETRY_COUNT + 1))/$MAX_RETRIES..."

    $FLASH_CMD

    if [ $? -eq 0 ]; then
        echo "✅ Flash successful!"
        exit 0
    fi

    RETRY_COUNT=$((RETRY_COUNT + 1))

    if [ $RETRY_COUNT -lt $MAX_RETRIES ]; then
        echo "⚠️  Flash failed, waiting 2 seconds before retry..."
        sleep 2
    fi
done

echo "❌ Flash failed after $MAX_RETRIES attempts"
exit 1
