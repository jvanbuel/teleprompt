#!/bin/bash
# The app itself, driven like a person would under a virtual X display. The
# prompter is the page `teleprompt serve` serves, tested on its own in a
# browser (crates/teleprompt-cli/tests/page); this tests what the app adds
# around it: it launches the server and shows its page, gives the page the
# microphone (WebKit's own, which hears a tone), passes the record key to
# it, opens its screen in a window of its own, and takes the server with it
# when it goes. A second run hears a recorded reading of apps/fixtures/tour
# (TELEPROMPT_MIC) and keeps it as takes.
#
#   TELEPROMPT_BIN=…/teleprompt TELEPROMPT_MODEL=…/zipformer apps/linux/tests/ui.sh
#
# Needs xvfb-run, xdotool, ImageMagick and ffmpeg. Screenshots are left in
# $UI_SHOTS (default: a temporary directory).
set -euo pipefail
# Waits until the page is up: its next word, in the cue's amber, on the glass.
shown() {
  amber=0
  for _ in $(seq 120); do
    import -window root "$work/now.png" 2>/dev/null || true
    amber=$(convert "$work/now.png" -crop 900x500+0+100 +repage -fuzz 12% -fill red -opaque '#ffb800' \
      -fill black +opaque red -format '%[fx:mean.r>0.001]' info: 2>/dev/null || echo 0)
    [ "$amber" = 1 ] && return 0
    sleep 0.5
  done
  return 1
}
if [ "${1:-}" = --read ]; then
  export GSK_RENDERER=cairo GDK_BACKEND=x11 NO_AT_BRIDGE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1
  export XDG_CONFIG_HOME="$work/config" XDG_CACHE_HOME="$work/cache" TELEPROMPT_MIC="$work/reading.wav"
  "$app" "$work/project/scripts/tour.md" >"$work/app2.log" 2>&1 &
  pid=$!
  shown || { echo "the prompter never showed"; cat "$work/app2.log"; exit 1; }
  xdotool mousemove 640 700 click 1
  xdotool key ctrl+shift+space
  for i in $(seq -w 1 20); do
    sleep 0.5
    import -window root "$shots/1$i-reading.png"
  done
  xdotool key ctrl+shift+space
  sleep 3
  import -window root "$shots/199-kept.png"
  kill "$pid"
  wait "$pid" || true
  exit 0
fi
if [ "${1:-}" = --drive ]; then
  # Inside xvfb-run, with $app, $work and $shots set by the outer run.
  export GSK_RENDERER=cairo GDK_BACKEND=x11 NO_AT_BRIDGE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1
  export XDG_CONFIG_HOME="$work/config" XDG_CACHE_HOME="$work/cache" TELEPROMPT_MOCK_MIC=1
  "$app" "$work/project/scripts/tour.md" >"$work/app.log" 2>&1 &
  pid=$!
  echo "$pid" >"$work/app.pid"
  shown || { import -window root "$shots/00-never.png"; echo "the prompter never showed"; cat "$work/app.log"; exit 1; }
  import -window root "$shots/00-opened.png"
  xdotool search --name 'Teleprompt' getwindowname %@ 2>/dev/null | grep '^tour\.md' >"$work/titled" || true
  # The record key, from the app's window to the page: on air, the glass
  # is framed in red.
  xdotool mousemove 640 700 click 1
  xdotool key ctrl+shift+space
  sleep 3
  import -window root "$shots/01-on-air.png"
  xdotool key Escape
  sleep 1.5
  import -window root "$shots/02-discarded.png"
  # The page's monitor, in a window of its own.
  xdotool mousemove 1205 404 click 1
  sleep 3
  xdotool search --name 'Teleprompt' getwindowname %@ 2>/dev/null | grep 'Screen$' >"$work/screen" || true
  import -window root "$shots/03-screen.png"
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
# A microphone does not stop at the last word: room tone after it.
ffmpeg -v error -i "$repo/crates/teleprompt-listen-sherpa/tests/fixtures/two-lines.wav" \
  -af apad=pad_dur=3 "$work/reading.wav"

export app work shots
xvfb-run -a -s "-screen 0 1400x860x24" "$0" --drive

fail=0
[ -s "$work/titled" ] && echo "ok: the window is named for the script" \
  || { echo "FAIL: the window is not named for the script"; fail=1; }
# The on-air frame, at the glass's left edge.
red() { convert "$1" -crop 1x1+8+400 -format "%[fx:r>0.8&&g<0.4]" info:; }
if [ "$(red "$shots/01-on-air.png")" = 1 ] && [ "$(red "$shots/02-discarded.png")" = 0 ]; then
  echo "ok: the record key reached the page, which had the microphone, and Escape ended the take"
else
  echo "FAIL: no take went on air, or it did not end"; tail -5 "$work/app.log"; fail=1
fi
[ -s "$work/screen" ] && echo "ok: the screen opened in a window of its own" \
  || { echo "FAIL: no window for the screen"; fail=1; }
xvfb-run -a -s "-screen 0 1400x860x24" "$0" --read
for line in welcome deploy; do
  if [ -f "$work/project/takes/$line.json" ]; then
    echo "ok: $line, heard in the app, was kept"
  else
    echo "FAIL: $line was not kept"; tail -5 "$work/app2.log"; fail=1
  fi
done
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
