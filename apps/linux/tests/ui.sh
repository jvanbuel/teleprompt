#!/bin/bash
# The app itself, driven like a person would: under a virtual X display,
# with a recorded reading of apps/fixtures/tour as its microphone. It opens
# the script, starts a take with Ctrl+T, keeps it with Return, and checks
# that both lines were saved as takes and that a shot's clip was on screen.
#
#   TELEPROMPT_BIN=…/teleprompt TELEPROMPT_MODEL=…/zipformer apps/linux/tests/ui.sh
#
# Needs xvfb-run, xdotool, ImageMagick and ffmpeg. Screenshots are left in
# $UI_SHOTS (default: a temporary directory), one every half second.
set -euo pipefail
if [ "${1:-}" = --drive ]; then
  # Inside xvfb-run, with $app, $work and $shots set by the outer run.
  export GSK_RENDERER=cairo GDK_BACKEND=x11 NO_AT_BRIDGE=1
  export XDG_CONFIG_HOME="$work/config" XDG_CACHE_HOME="$work/cache"
  export TELEPROMPT_MIC="filesrc location=$work/reading.wav ! wavparse"
  "$app" "$work/project/scripts/tour.md" >"$work/app.log" 2>&1 &
  pid=$!
  window=""
  for _ in $(seq 60); do
    window=$(xdotool search --name '^Teleprompt$' 2>/dev/null | head -1 || true)
    [ -n "$window" ] && break
    sleep 0.5
  done
  [ -n "$window" ] || { echo "no window"; cat "$work/app.log"; exit 1; }
  sleep 4 # the server loads the model
  xdotool windowfocus --sync "$window"
  import -window root "$shots/00-opened.png"
  xdotool key ctrl+t
  for i in $(seq -w 1 22); do
    sleep 0.5
    import -window root "$shots/$i-reading.png"
  done
  xdotool key Return
  sleep 2
  import -window root "$shots/99-kept.png"
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

cp -r "$repo/apps/fixtures/tour" "$work/project"
(cd "$work/project" && "$TELEPROMPT_BIN" capture scripts/tour.md >/dev/null)
# A microphone does not stop at the last word: two seconds of room tone,
# so the recognizer hears the reading end.
ffmpeg -v error -i "$repo/crates/teleprompt-listen-sherpa/tests/fixtures/two-lines.wav" \
  -af apad=pad_dur=2 "$work/reading.wav"

export app work shots
xvfb-run -a -s "-screen 0 1400x860x24" "$0" --drive

fail=0
for line in welcome deploy; do
  if [ -f "$work/project/takes/$line.json" ]; then
    echo "ok: $line kept ($(grep -o '"duration_ms": [0-9]*' "$work/project/takes/$line.json"))"
  else
    echo "FAIL: $line was not kept"; fail=1
  fi
done
# The screen is the right-hand pane; a clip on it is anything but black.
lit=0
for shot in "$shots"/*-reading.png; do
  mean=$(convert "$shot" -crop 300x200+900+280 -format "%[fx:mean]" info:)
  if awk "BEGIN { exit !($mean > 0.1) }"; then lit=1; break; fi
done
if [ "$lit" = 1 ]; then echo "ok: a clip was on screen ($(basename "$shot"))"; else echo "FAIL: no clip was ever on screen"; fail=1; fi
# The app was killed, not closed: its server must have gone with it.
for _ in $(seq 10); do
  pgrep -f "$work/project/scripts/tour.md" >/dev/null || break
  sleep 0.5
done
if pgrep -f "$work/project/scripts/tour.md" >/dev/null; then
  echo "FAIL: the server outlived the app"; fail=1
else
  echo "ok: the server went with the app"
fi
exit $fail
