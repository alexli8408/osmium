#!/usr/bin/env bash
# Headless boot test: boot the kernel under QEMU, capture the serial console,
# and wait for the in-kernel boot self-tests to report success.
set -u

QEMU="$1"
QEMU_FLAGS="$2"
KERNEL="$3"
# Portable across GNU (Linux/CI) and BSD (macOS) mktemp.
LOG="$(mktemp "${TMPDIR:-/tmp}/osmium-boot.XXXXXX")"
TIMEOUT_SECS=30
MARKER="ALL BOOT TESTS PASSED"

echo "booting $KERNEL (log: $LOG)"

$QEMU $QEMU_FLAGS -kernel "$KERNEL" >"$LOG" 2>&1 </dev/null &
QPID=$!

PASS=0
for _ in $(seq 1 $((TIMEOUT_SECS * 2))); do
    if grep -q "$MARKER" "$LOG"; then
        PASS=1
        break
    fi
    if ! kill -0 "$QPID" 2>/dev/null; then
        break # QEMU exited on its own
    fi
    sleep 0.5
done

kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

echo "--- console output ---"
cat "$LOG"
echo "----------------------"

# QEMU may have exited before the poll loop saw the marker; check once more.
if [ "$PASS" -eq 0 ] && grep -q "$MARKER" "$LOG"; then
    PASS=1
fi

if [ "$PASS" -eq 1 ]; then
    echo "boot-test: PASS"
    exit 0
else
    echo "boot-test: FAIL ('$MARKER' not seen within ${TIMEOUT_SECS}s)"
    exit 1
fi
