# The app icon

`teleprompt.svg` is a slice of the prompter's glass: black, the amber cue
arrow on the reading line, a line said, the line being read, a line to come.
The Linux app embeds it; `apps/macos/AppIcon.iconset` is rendered from it
(`rsvg-convert`), and `apps/macos/bundle.sh` turns that into the `.icns`.
