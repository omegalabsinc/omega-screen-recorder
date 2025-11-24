#!/bin/bash

# Test script to verify timer behavior when recorder is spawned and killed multiple times
# This tests that the cumulated time across multiple recording sessions is accurate

set -e  # Exit on error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
TEST_TASK_ID="test-timer-$(date +%s)"
# Variable session durations: 2min, 4min, 5min (total: 11min expected)
RECORDING_DURATIONS=(120 240 300)  # Durations in seconds for each session
NUM_SESSIONS=${#RECORDING_DURATIONS[@]}  # Number of sessions based on array length
RECORDER_BIN="./target/release/omgrec"
DB_PATH="$HOME/.omega/db.sqlite"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}Timer Behavior Test Script${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo -e "Test Task ID: ${YELLOW}${TEST_TASK_ID}${NC}"
echo -e "Session Durations: ${YELLOW}2min, 4min, 5min (120s, 240s, 300s)${NC}"
echo -e "Number of Sessions: ${YELLOW}${NUM_SESSIONS}${NC}"
echo -e "Total Expected Time: ${YELLOW}11 minutes (660 seconds)${NC}"
echo -e "Recorder Binary: ${YELLOW}${RECORDER_BIN}${NC}"
echo ""

# Check if recorder binary exists
if [ ! -f "$RECORDER_BIN" ]; then
    echo -e "${RED}Error: Recorder binary not found at ${RECORDER_BIN}${NC}"
    echo -e "${YELLOW}Building the project in release mode...${NC}"
    cargo build --release
fi

# Array to store actual session durations
declare -a ACTUAL_DURATIONS

# Function to cleanup on exit
cleanup() {
    echo -e "\n${YELLOW}Cleaning up...${NC}"
    # Kill any remaining recorder processes
    pkill -f "omgrec record" || true
    echo -e "${GREEN}Cleanup complete${NC}"
}

trap cleanup EXIT

# Function to start a recording session
start_recording() {
    local session_num=$1
    local duration=$2
    local duration_min=$((duration / 60))
    echo -e "\n${BLUE}[Session ${session_num}/${NUM_SESSIONS}] Starting recording...${NC}"
    echo -e "${BLUE}  Duration: ${duration}s (${duration_min} minutes)${NC}"

    # Record the start time
    local start_time=$(date +%s)

    # Start recording in background
    "$RECORDER_BIN" record \
        --recording-type task \
        --task-id "$TEST_TASK_ID" \
        --fps 30 \
        --no-audio \
        --chunk-duration 10 \
        > /dev/null 2>&1 &

    local recorder_pid=$!
    echo -e "${GREEN}Started recorder with PID: ${recorder_pid}${NC}"

    # Wait for the specified duration with progress updates
    echo -e "${YELLOW}Recording for ${duration}s (${duration_min}m)...${NC}"

    # Show progress every 30 seconds for long recordings
    local elapsed=0
    while [ $elapsed -lt $duration ]; do
        local remaining=$((duration - elapsed))
        if [ $remaining -gt 30 ]; then
            sleep 30
            elapsed=$((elapsed + 30))
            echo -e "${YELLOW}  Progress: ${elapsed}s / ${duration}s (${remaining}s remaining)${NC}"
        else
            sleep $remaining
            elapsed=$duration
        fi
    done

    # Kill the recorder
    echo -e "${YELLOW}Stopping recorder...${NC}"
    kill -TERM "$recorder_pid" 2>/dev/null || true

    # Wait for process to terminate gracefully
    local wait_count=0
    while kill -0 "$recorder_pid" 2>/dev/null && [ $wait_count -lt 10 ]; do
        sleep 0.5
        wait_count=$((wait_count + 1))
    done

    # Force kill if still running
    if kill -0 "$recorder_pid" 2>/dev/null; then
        echo -e "${RED}Force killing recorder...${NC}"
        kill -9 "$recorder_pid" 2>/dev/null || true
    fi

    # Calculate actual duration
    local end_time=$(date +%s)
    local actual_duration=$((end_time - start_time))
    ACTUAL_DURATIONS+=($actual_duration)

    echo -e "${GREEN}[Session ${session_num}/${NUM_SESSIONS}] Recording stopped (actual duration: ${actual_duration}s)${NC}"

    # Give database time to finalize writes
    sleep 1
}

# Main test execution
echo -e "\n${BLUE}========================================${NC}"
echo -e "${BLUE}Running Recording Sessions${NC}"
echo -e "${BLUE}========================================${NC}"

for i in $(seq 0 $((NUM_SESSIONS - 1))); do
    session_num=$((i + 1))
    duration=${RECORDING_DURATIONS[$i]}

    start_recording $session_num $duration

    # Add a small gap between sessions
    if [ $session_num -lt $NUM_SESSIONS ]; then
        echo -e "${YELLOW}Waiting 5 seconds before next session...${NC}"
        sleep 5
    fi
done

# Wait a bit for database to settle
echo -e "\n${YELLOW}Waiting for database to settle...${NC}"
sleep 2

# Verification Phase
echo -e "\n${BLUE}========================================${NC}"
echo -e "${BLUE}Verification Phase${NC}"
echo -e "${BLUE}========================================${NC}"

# Use the inspect-sessions command to check recorded times
echo -e "\n${YELLOW}Inspecting recorded sessions:${NC}"
"$RECORDER_BIN" inspect-sessions --task-id "$TEST_TASK_ID"

# Query database directly for more detailed verification
echo -e "\n${YELLOW}Querying database directly...${NC}"

if [ -f "$DB_PATH" ]; then
    # Get session details
    echo -e "\n${BLUE}Session Details:${NC}"
    sqlite3 "$DB_PATH" <<EOF
.headers on
.mode column
SELECT
    id,
    started_at,
    ended_at,
    ROUND((julianday(ended_at) - julianday(started_at)) * 86400, 2) as duration_seconds
FROM recording_sessions
WHERE task_id = '$TEST_TASK_ID'
ORDER BY started_at;
EOF

    # Get total time
    echo -e "\n${BLUE}Total Recording Time:${NC}"
    total_db_time=$(sqlite3 "$DB_PATH" "SELECT ROUND(SUM((julianday(ended_at) - julianday(started_at)) * 86400), 2) FROM recording_sessions WHERE task_id = '$TEST_TASK_ID' AND ended_at IS NOT NULL;")
    echo -e "Database Total: ${GREEN}${total_db_time}${NC} seconds"

    # Calculate expected time by summing the array
    total_expected_time=0
    for duration in "${RECORDING_DURATIONS[@]}"; do
        total_expected_time=$((total_expected_time + duration))
    done
    total_expected_min=$((total_expected_time / 60))
    echo -e "Expected Total: ${YELLOW}${total_expected_time}${NC} seconds (${total_expected_min} minutes: 2m + 4m + 5m)"

    # Calculate actual time from our measurements
    total_actual_time=0
    for duration in "${ACTUAL_DURATIONS[@]}"; do
        total_actual_time=$((total_actual_time + duration))
    done
    echo -e "Measured Total: ${YELLOW}${total_actual_time}${NC} seconds"

    # Calculate differences
    db_vs_expected=$(echo "$total_db_time - $total_expected_time" | bc)
    db_vs_actual=$(echo "$total_db_time - $total_actual_time" | bc)

    echo -e "\n${BLUE}Analysis:${NC}"
    echo -e "Difference (DB vs Expected): ${YELLOW}${db_vs_expected}${NC} seconds"
    echo -e "Difference (DB vs Measured): ${YELLOW}${db_vs_actual}${NC} seconds"

    # Check if times are within acceptable range
    # Note: Measured time includes process startup/shutdown overhead (~2-3s per session)
    # For longer sessions (minutes), we also allow 1.5% variance for timing accuracy
    # We use a tolerance that accounts for both overhead and timing variance
    tolerance_per_session=2.5
    variance_tolerance=$(echo "scale=2; $total_expected_time * 0.015" | bc)  # 1.5% total variance
    total_tolerance=$(echo "$NUM_SESSIONS * $tolerance_per_session + $variance_tolerance" | bc)
    db_vs_actual_abs=$(echo "$db_vs_actual" | awk '{print ($1 < 0) ? -$1 : $1}')

    if (( $(echo "$db_vs_actual_abs < $total_tolerance" | bc -l) )); then
        echo -e "\n${GREEN}✓ PASS: Database time is reasonable considering startup overhead${NC}"
        echo -e "${GREEN}  (Tolerance: ${total_tolerance}s for ${NUM_SESSIONS} sessions)${NC}"
        exit_code=0
    else
        echo -e "\n${RED}✗ FAIL: Database time differs significantly from measured time (>${total_tolerance}s difference)${NC}"
        exit_code=1
    fi

    # Check for incomplete sessions
    incomplete_count=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM recording_sessions WHERE task_id = '$TEST_TASK_ID' AND ended_at IS NULL;")
    if [ "$incomplete_count" -gt 0 ]; then
        echo -e "${RED}⚠ WARNING: Found ${incomplete_count} incomplete session(s) without end time${NC}"
        exit_code=1
    fi

else
    echo -e "${RED}Error: Database file not found at ${DB_PATH}${NC}"
    exit_code=1
fi

# Summary
echo -e "\n${BLUE}========================================${NC}"
echo -e "${BLUE}Test Summary${NC}"
echo -e "${BLUE}========================================${NC}"
echo -e "Test Task ID: ${YELLOW}${TEST_TASK_ID}${NC}"
echo -e "Sessions Run: ${YELLOW}${NUM_SESSIONS}${NC} (2min, 4min, 5min)"
echo -e "Expected Duration: ${YELLOW}${total_expected_time}s (${total_expected_min} minutes)${NC}"
echo -e "Measured Duration: ${YELLOW}${total_actual_time}s ($((total_actual_time / 60)) minutes)${NC}"
db_time_min=$(echo "scale=1; $total_db_time / 60" | bc)
echo -e "Database Duration: ${GREEN}${total_db_time}s (${db_time_min} minutes)${NC}"

if [ $exit_code -eq 0 ]; then
    echo -e "\n${GREEN}✓ All tests passed!${NC}"
else
    echo -e "\n${RED}✗ Some tests failed!${NC}"
fi

echo -e "\n${YELLOW}Note: Test data remains in database. Clean up with:${NC}"
echo -e "sqlite3 $DB_PATH \"DELETE FROM recording_sessions WHERE task_id = '$TEST_TASK_ID'; DELETE FROM video_chunks WHERE task_id = '$TEST_TASK_ID';\""

exit $exit_code
