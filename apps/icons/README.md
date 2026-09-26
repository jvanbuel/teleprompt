# The app icon

`teleprompt.svg` is a slice of the prompter's glass: black, the amber cue
arrow on the reading line, a line said, the line being read, a line to come.
The Linux app embeds it; `apps/macos/AppIcon.iconset` is rendered from it
(`rsvg-convert`), and `apps/macos/bundle.sh` turns that into the `.icns`.

## The name

`teleprompt-lockup-dark.svg` (for dark backgrounds) and
`teleprompt-lockup-light.svg` put the icon beside the name;
`teleprompt-wordmark.svg` is the name alone. The name is set in Space Mono
Bold (Colophon Foundry, OFL), lowercase, with the o replaced by the font's
dotted zero in cue amber: the one amber thing, as on the glass. The letters
are outlines, so the files need no font. `lockup.py` draws them.
