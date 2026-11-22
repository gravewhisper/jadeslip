#!/usr/bin/env bash
S=$(cd "$(dirname "$0")" && pwd)
plan=$1
log=$2
while true; do
    gone=0
    while [ "$gone" -lt 3 ]; do
        if pgrep -x dota2 >/dev/null; then gone=0; else gone=$((gone + 1)); fi
        [ "$gone" -lt 3 ] && sleep 60
    done
    echo "$(date +%T) start $plan" >> "$S/gate.log"
    setsid bash "$S/run-plan.sh" "$plan" "$log" &
    runner=$!
    gamed=0
    while kill -0 "$runner" 2>/dev/null; do
        sleep 60 &
        wait $! 
        if kill -0 "$runner" 2>/dev/null && pgrep -x dota2 >/dev/null; then
            gamed=1
            pkill -TERM -f "headless.sh python3 $S/measure.py $plan"
            wait "$runner"
        fi
    done
    wait "$runner"
    if [ "$gamed" = 0 ]; then
        echo "$(date +%T) done $plan" >> "$S/gate.log"
        exit 0
    fi
    echo "$(date +%T) stopped $plan for dota2" >> "$S/gate.log"
    exit 3
done
