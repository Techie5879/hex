#!/usr/bin/env bash
# Whole-pipeline check on an emulator started with QEMU_AUDIO_DRV=wav: its qemu_in.wav reaches
# AudioRecord through the virtual microphone, the real overlay is tapped, and the text must land in a real
# field. See docs/testing.md. Software emulation is slow: this proves behaviour, not speed.
#
# usage: e2e-dictation.sh <the same speech.wav> <expected-word> [hold]
set -euo pipefail
wav=$1
expected=$2
mode=${3:-tap}
adb=${ADB:-adb}  # select the device with ANDROID_SERIAL
timeout=${E2E_TIMEOUT:-1800}

state() { "$adb" logcat -d -s HexDictation:I | grep 'state ' | tail -1 | awk '{print $NF}'; }

bubble_center() {
    "$adb" shell dumpsys window windows | grep -A28 'Window #.*HexBubble' | grep -m1 -oE ' frame=\[[-0-9]+,[-0-9]+\]\[[-0-9]+,[-0-9]+\]' |
        sed -E 's/ frame=\[([-0-9]+),([-0-9]+)\]\[([-0-9]+),([-0-9]+)\]/\1 \2 \3 \4/' || true
}

wait_for() {
    local want=$1 deadline=$((SECONDS + timeout))
    until [ "$(state)" = "$want" ]; do
        [ "$SECONDS" -lt "$deadline" ] || { echo "timed out waiting for $want, at $(state)"; return 1; }
        sleep 2
    done
}

echo "focusing a multiline field in the test app"
"$adb" logcat -c
"$adb" shell am start -W -n anomaly.hex.android.testapp/.FieldsActivity >/dev/null
sleep 5
"$adb" shell input tap 540 180
for _ in $(seq 60); do
    [ -n "$(bubble_center)" ] && break
    sleep 2
done
read -r left top right bottom < <(bubble_center)
x=$(((left + right) / 2))
y=$(((top + bottom) / 2))
echo "bubble at $x,$y"
started=$SECONDS
seconds=$(soxi -D "$wav" | cut -d. -f1)
seconds=$((seconds + 2))

if [ "$mode" = hold ]; then
    # A long press from the shell is a swipe that doesn't move.
    duration=$(((seconds + 3) * 1000))
    "$adb" shell input swipe "$x" "$y" "$x" "$y" "$duration" &
    press=$!
    wait_for Recording
    "$adb" emu avd rewindaudio >/dev/null
    wait "$press"
else
    "$adb" shell input tap "$x" "$y"
    wait_for Recording
    "$adb" emu avd rewindaudio >/dev/null
    sleep "$seconds"
    read -r left top right bottom < <(bubble_center)
    if [ "$(state)" = Recording ]; then
        # In hands-free recording the handle, which is ✓, stays at the screen edge.
        edge=$((right - (bottom - top) / 2))
        "$adb" shell input tap "$edge" $(((top + bottom) / 2))
    fi
fi

echo "processing"
deadline=$((SECONDS + timeout))
until "$adb" logcat -d -s HexDictation:I | tail -5 | grep -qE 'state (Inserted|ResultReady|Failed)'; do
    [ "$SECONDS" -lt "$deadline" ] || { echo "timed out in $(state)"; exit 1; }
    sleep 3
done
echo "finished in $((SECONDS - started)) s, state $(state). Emulator time says nothing about a phone."
[ "$(state)" = Inserted ] || "$adb" logcat -d -s HexDictation:I | tail -3
"$adb" exec-out screencap -p >"${E2E_SHOT:-/tmp/hex-e2e.png}"
dump=$("$adb" exec-out uiautomator dump /dev/stdout 2>/dev/null || true)
if grep -qi "$expected" <<<"$dump"; then
    echo "PASS: the field contains '$expected'"
else
    echo "FAIL: '$expected' not found on screen; see ${E2E_SHOT:-/tmp/hex-e2e.png}"
    exit 1
fi
