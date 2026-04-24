#!/usr/bin/env bash
# watchdog.sh — turns bench stalls into clean failure notifications.
#
# Usage: watchdog.sh <jsonl_path> [stall_seconds]
#
# Polls the bench's JSONL output every 30s. If its mtime hasn't advanced
# in [stall_seconds] (default 300 = 5 min) AND the bench process is
# still alive, SIGTERM the bench. The dying bench triggers Claude
# Code's existing background-task failure notification, which is the
# only channel we can use to surface stalls.
#
# Normal rollout pacing is 45-60s/rollout — 5 min is a genuine stall,
# not a slow-but-healthy rollout. Tune with the second argument.
#
# Exits 0 when the bench exits naturally. Exits 1 when it kills due to
# stall, with the reason logged to the sibling .watchdog.log file.

set -u

JSONL="${1:?usage: watchdog.sh <jsonl_path> [stall_seconds]}"
STALL_SECS="${2:-300}"
POLL_SECS=30
LOG="${JSONL%.jsonl}.watchdog.log"

log() { printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "$LOG" ; }

# Wait up to 60s for the JSONL file to appear (bench startup can take
# a moment for cold model reload + cache clear).
for _ in $(seq 1 60); do
    [ -f "$JSONL" ] && break
    sleep 1
done
if [ ! -f "$JSONL" ]; then
    log "JSONL never appeared; giving up without action"
    exit 0
fi
log "watchdog engaged on $JSONL (stall threshold ${STALL_SECS}s)"

last_mtime=$(stat -c %Y "$JSONL")
last_change=$(date +%s)

while :; do
    # Is the bench still alive?
    if ! pgrep -f "bench.microbench" > /dev/null 2>&1; then
        log "bench process exited; watchdog done"
        exit 0
    fi

    sleep "$POLL_SECS"

    now=$(date +%s)
    cur_mtime=$(stat -c %Y "$JSONL" 2>/dev/null || echo 0)
    if [ "$cur_mtime" -ne "$last_mtime" ]; then
        last_mtime=$cur_mtime
        last_change=$now
        continue
    fi

    idle=$(( now - last_change ))
    if [ "$idle" -ge "$STALL_SECS" ]; then
        lines=$(wc -l < "$JSONL" 2>/dev/null || echo 0)
        log "STALL: no JSONL append for ${idle}s (threshold ${STALL_SECS}s) at line $lines — killing bench"
        pkill -TERM -f "bench.microbench" 2>/dev/null || true
        # Give it a moment, then SIGKILL if still alive
        sleep 5
        if pgrep -f "bench.microbench" > /dev/null 2>&1; then
            log "SIGTERM ignored; sending SIGKILL"
            pkill -KILL -f "bench.microbench" 2>/dev/null || true
        fi
        exit 1
    fi
done
