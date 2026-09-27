#!/bin/bash
# The app itself, driven like a person would: under a virtual X display,
# with a recorded reading of apps/fixtures/tour as its microphone. It opens
# the script, starts and keeps a take with Ctrl+Shift+Space, and checks
# that both lines were saved as takes and that a shot's clip was on screen.
#
#   TELEPROMPT_BIN=…/teleprompt TELEPROMPT_MODEL=…/zipformer apps/linux/tests/ui.sh
#
# Needs xvfb-run, xdotool, ImageMagick and ffmpeg. Screenshots are left in
# $UI_SHOTS (default: a temporary directory), one every half second.
set -euo pipefail
if [ "${1:-}" = --retake ]; then
  # A second run, with the takes the first kept: a line reworded while the
  # script is open is re-recorded on its own, with R.
  export GSK_RENDERER=cairo GDK_BACKEND=x11 NO_AT_BRIDGE=1
  export XDG_CONFIG_HOME="$work/config" XDG_CACHE_HOME="$work/cache"
  export TELEPROMPT_MIC="filesrc location=$work/reading.wav ! wavparse"
  "$app" "$work/project/scripts/tour.md" >"$work/app2.log" 2>&1 &
  pid=$!
  for _ in $(seq 120); do
    red=$(import -window root -crop 1x1+1030+27 -format "%[fx:r>0.8&&g<0.4]" info: 2>/dev/null || echo 0)
    [ "$red" = 1 ] && break
    sleep 0.5
  done
  window=$(xdotool search --name '^Teleprompt$' | head -1)
  xdotool windowfocus --sync "$window"
  sed -i 's/Let me show you around\./Let me show you around!/' "$work/project/scripts/tour.md"
  sleep 3
  import -window root "$shots/200-reworded.png"
  xdotool key r
  for i in $(seq -w 1 24); do
    sleep 0.5
    import -window root "$shots/2$i-retaking.png"
  done
  kill "$pid"
  wait "$pid" || true
  exit 0
fi
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
  # The server loads the model; the red Record button shows when it has.
  for _ in $(seq 120); do
    red=$(import -window root -crop 1x1+1030+27 -format "%[fx:r>0.8&&g<0.4]" info: 2>/dev/null || echo 0)
    [ "$red" = 1 ] && break
    sleep 0.5
  done
  [ "$red" = 1 ] || { import -window root "$shots/never-ready.png"; echo "the prompter never became ready"; cat "$work/app.log"; exit 1; }
  xdotool windowfocus --sync "$window"
  import -window root "$shots/00-opened.png"
  xdotool key ctrl+shift+space
  # The count of three, the reading, and the room tone after it.
  for i in $(seq -w 1 30); do
    sleep 0.5
    import -window root "$shots/$i-reading.png"
  done
  xdotool key ctrl+shift+space
  sleep 2
  import -window root "$shots/99-kept.png"
  # The glass as the timeline: in Edit mode, a word hovered shows on the
  # monitor what is on screen as it is said.
  xdotool key e
  xdotool mousemove 620 323
  sleep 2
  import -window root "$shots/99-scrubbed.png"
  # The timeline: drags edit the script. Pressed, moved in steps as a hand
  # moves, and let go.
  drag() {
    local to_y=${4:-$2}
    xdotool mousemove "$1" "$2" mousedown 1
    for i in $(seq 1 12); do
      xdotool mousemove $(( $1 + ($3 - $1) * i / 12 )) $(( $2 + (to_y - $2) * i / 12 ))
      sleep 0.03
    done
    xdotool mouseup 1
    sleep 2.5
  }
  md="$work/project/scripts/tour.md"
  # The second shot's end, pulled left: shorter.
  drag 1255 708 1150
  cp "$md" "$work/stretched.md"
  import -window root "$shots/100-stretched.png"
  # The first shot, onto its own line: it starts on a word there.
  drag 560 708 250
  cp "$md" "$work/cued.md"
  import -window root "$shots/101-cued.png"
  # And undone.
  xdotool key ctrl+z
  sleep 1.5
  cp "$md" "$work/undone.md"
  # The first shot's pill, in the pause after its line, onto "show".
  drag 400 167 740 103
  cp "$md" "$work/glass.md"
  import -window root "$shots/101-glass.png"
  # Build: the shots the drags changed are captured again, then the video.
  xdotool key ctrl+b
  sleep 1
  import -window root "$shots/102-building.png"
  for _ in $(seq 120); do
    [ -f "$work/project/build/tour.en.mp4" ] && break
    sleep 0.5
  done
  sleep 1.5
  import -window root "$shots/103-built.png"
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
# The monitor is the top of the right-hand pane; a clip on it is anything
# but black.
lit=0
for shot in "$shots"/*-reading.png; do
  mean=$(convert "$shot" -crop 280x140+940+100 -format "%[fx:mean]" info:)
  if awk "BEGIN { exit !($mean > 0.1) }"; then lit=1; break; fi
done
if [ "$lit" = 1 ]; then echo "ok: a clip was on screen ($(basename "$shot"))"; else echo "FAIL: no clip was ever on screen"; fail=1; fi
# The timeline's drags, written into the script and undone.
if grep -q 'cue="streams progress" stretch=0\.' "$work/stretched.md"; then
  echo "ok: a shot's end dragged is a stretch"
else
  echo "FAIL: no stretch"; cat "$work/stretched.md"; fail=1
fi
if grep -Eq '^```teleprompt scene=mock policy=concurrent cue=("[^"]*"|[^ ]+)$' "$work/cued.md" \
  && [ "$(grep -c 'cue=' "$work/cued.md")" = 2 ]; then
  echo "ok: a shot dragged onto its line is cued there"
else
  echo "FAIL: no cue"; cat "$work/cued.md"; fail=1
fi
if cmp -s "$work/stretched.md" "$work/undone.md"; then
  echo "ok: Ctrl+Z undoes the last drag"
else
  echo "FAIL: not undone"; diff "$work/stretched.md" "$work/undone.md"; fail=1
fi
kept=$(convert "$shots/99-kept.png" -crop 280x140+940+100 -format "%[fx:mean]" info:)
scrubbed=$(convert "$shots/99-scrubbed.png" -crop 280x140+940+100 -format "%[fx:mean]" info:)
if awk "BEGIN { exit !($kept < 0.1 && $scrubbed > 0.1) }"; then
  echo "ok: a word hovered in Edit mode shows its moment on the monitor"
else
  echo "FAIL: no scrub ($kept, then $scrubbed)"; fail=1
fi
if grep -Eq '^```teleprompt scene=mock policy=concurrent cue="show you"$' "$work/glass.md"; then
  echo "ok: a shot's pill dragged onto a word of its line on the glass is cued there"
else
  echo "FAIL: no cue from the glass"; cat "$work/glass.md"; fail=1
fi
if [ -s "$work/project/build/tour.en.mp4" ]; then
  echo "ok: Ctrl+B built the video"
else
  echo "FAIL: no video built"; fail=1
fi
xvfb-run -a -s "-screen 0 1400x860x24" "$0" --retake
if grep -q 'show you around!' "$work/project/takes/welcome.json" \
  && grep -q 'Deployment is one command' "$work/project/takes/deploy.json"; then
  echo "ok: R re-recorded the reworded line, and only it"
else
  echo "FAIL: the reworded line was not re-recorded"; cat "$work/project/takes/"*.json; tail -5 "$work/app2.log"; fail=1
fi
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
