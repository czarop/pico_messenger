#!/usr/bin/env bash
# Re-run `just attach` forever, reconnecting whenever the probe drops.
#
# probe-rs exits (nonzero) when the CMSIS-DAP/USB link faults; this loop just
# runs it again. All RTT output is also tee'd to a timestamped log file so a
# rare event (e.g. a teardown that ends in a reset) is captured even if you
# weren't watching when the probe was up.
#
# Usage:   ./reattach.sh
# Stop:    Ctrl-C
# Grep later, e.g.:
#   grep -n "MQTT state before teardown\|MQTTDISC did not confirm" rtt-*.log

set -u

LOG="rtt-$(date +%Y%m%d-%H%M%S).log"
DELAY="${REATTACH_DELAY:-2}"   # seconds between attempts; override: REATTACH_DELAY=5 ./reattach.sh

# Clean exit on Ctrl-C rather than instantly relaunching.
trap 'echo; echo "[reattach] stopped by user. Log: $LOG"; exit 0' INT TERM

echo "[reattach] logging to $LOG"
echo "[reattach] Ctrl-C to stop"

attempt=0
while true; do
    attempt=$((attempt + 1))
    ts="$(date '+%Y-%m-%d %H:%M:%S')"
    echo "[reattach] --- attempt #$attempt at $ts ---" | tee -a "$LOG"

    # Run attach; mirror stdout+stderr to console and log.
    # `just attach` runs: probe-rs attach --chip RP2350 <elf>
    just attach 2>&1 | tee -a "$LOG"

    # If we get here, attach exited (probe dropped or was detached).
    echo "[reattach] attach exited, retrying in ${DELAY}s..." | tee -a "$LOG"
    sleep "$DELAY"
done