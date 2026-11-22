#!/usr/bin/env bash
while kill -0 "$1" 2>/dev/null; do sleep 0.5; done
for pid in /proc/[0-9]*; do
    if grep -qz '^SS_SPEED_RUN=1$' "$pid/environ" 2>/dev/null; then kill -KILL "${pid#/proc/}" 2>/dev/null; fi
done
hyprctl eval "if SS_CALIBRE_RULE then SS_CALIBRE_RULE:set_enabled(false) end" >/dev/null
