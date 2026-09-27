#!/bin/bash
# Session mode, driven like a person would under a virtual X display: it
# names a new script, presses Ctrl+Shift+Space to record, types `ls` into the
# terminal between two readings of the recognizer's fixture (played as the
# microphone), presses it again to stop, and checks the draft and its
# takes, and that the draft opened in the prompter.
#
#   TELEPROMPT_BIN=…/teleprompt TELEPROMPT_MODEL=…/zipformer apps/linux/tests/session.sh
#
# Needs xvfb-run, xdotool, ImageMagick and ffmpeg. Screenshots are left in
# $UI_SHOTS (default: a temporary directory).
set -euo pipefail
if [ "${1:-}" = --drive ]; then
  export GSK_RENDERER=cairo GDK_BACKEND=x11 NO_AT_BRIDGE=1
  export XDG_CONFIG_HOME="$work/config" XDG_CACHE_HOME="$work/cache"
  export TELEPROMPT_RECORD_MIC="-re -i $work/voice.wav"
  "$app" >"$work/app.log" 2>&1 &
  pid=$!
  window=""
  for _ in $(seq 60); do
    window=$(xdotool search --name '^Teleprompt$' 2>/dev/null | head -1 || true)
    [ -n "$window" ] && break
    sleep 0.5
  done
  [ -n "$window" ] || { echo "no window"; cat "$work/app.log"; exit 1; }
  sleep 1
  xdotool windowfocus --sync "$window"
  import -window root "$shots/00-welcome.png"
  xdotool key ctrl+n
  sleep 1.5
  xdotool key ctrl+a
  xdotool type --delay 5 "$work/project/scripts/session.md"
  xdotool key Return
  # Long enough for the tools to be listed.
  sleep 3
  import -window root "$shots/01-idle.png"
  xdotool key ctrl+shift+space
  sleep "$reading"
  import -window root "$shots/02-recording.png"
  sleep 2
  xdotool type --delay 80 "ls"
  xdotool key Return
  sleep 1
  import -window root "$shots/03-typed.png"
  sleep "$reading"
  sleep 4
  xdotool key ctrl+shift+space
  for i in $(seq -w 1 40); do
    sleep 1
    [ -f "$work/project/scripts/session.md" ] && break
  done
  # The draft opens in the prompter, whose server loads the model first:
  # the red Record button shows when it has.
  for _ in $(seq 60); do
    red=$(import -window root -crop 1x1+1030+27 -format "%[fx:r>0.8&&g<0.4]" info: 2>/dev/null || echo 0)
    [ "$red" = 1 ] && break
    sleep 0.5
  done
  import -window root "$shots/04-drafted.png"
  kill "$pid"
  wait "$pid" || true
  exit 0
fi
: "${TELEPROMPT_BIN:?the teleprompt binary, built with --features listen}"
: "${TELEPROMPT_MODEL:?an unpacked sherpa-onnx streaming zipformer}"
repo=$(cd "$(dirname "$0")/../../.." && pwd)
app=${TELEPROMPT_GTK:-$repo/apps/linux/target/debug/teleprompt-gtk}
work=$(mktemp -d)
shots=${UI_SHOTS:-$work/shots}
mkdir -p "$shots"
trap 'rm -rf "$work"' EXIT
"$TELEPROMPT_BIN" new "$work/project" >/dev/null
fixture="$repo/crates/teleprompt-listen-sherpa/tests/fixtures/two-lines.wav"
# The reading, a pause for typing, the reading again, and room tone.
ffmpeg -v error -i "$fixture" -i "$fixture" -filter_complex \
  "[0]apad=pad_dur=5[a];[1]apad=pad_dur=3[b];[a][b]concat=n=2:v=0:a=1" "$work/voice.wav"
reading=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$fixture")
export app work shots reading
xvfb-run -a -s "-screen 0 1400x860x24" "$0" --drive

fail=0
md="$work/project/scripts/session.md"
if [ -f "$md" ]; then
  echo "ok: the session was drafted"
  cast="$work/project/scripts/recordings/session.cast"
  grep -q 'include=recordings/session.cast#' "$md" && grep -q '"i", *"l' "$cast" \
    && echo "ok: the typing is recorded and included" \
    || { echo "FAIL: no recording of ls"; cat "$md" "$cast"; fail=1; }
  grep -qi 'welcome to acme' "$md" && echo "ok: the reading is a line" || { echo "FAIL: no line"; cat "$md"; fail=1; }
else
  echo "FAIL: no script was drafted"; cat "$work/app.log"; fail=1
fi
takes=$(ls "$work/project/takes" 2>/dev/null | grep -c '\.wav$' || true)
[ "$takes" -ge 2 ] && echo "ok: $takes takes" || { echo "FAIL: $takes takes"; fail=1; }
# The draft opens in the prompter: its Record button is red.
red=$(convert "$shots/04-drafted.png" -crop 1x1+1030+27 -format "%[fx:r>0.8&&g<0.4]" info:)
[ "$red" = 1 ] && echo "ok: the draft opened in the prompter" || { echo "FAIL: the draft did not open"; fail=1; }
exit $fail
