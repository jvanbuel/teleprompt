# The prompter's design

The prompters (`apps/linux`, `apps/macos` and the page `teleprompt prompt`
serves) share one design. It comes from what a prompter is for: someone
alone, reading to a camera, often through beam-splitter glass. The next word
must be impossible to miss, and everything else must recede.

It borrows from broadcast prompting and the studio:

- **The glass.** The text field is true black, because black is what a
  beam-splitter does not reflect.
- **The reading line.** A fixed eyeline a little over a third down, with a
  cue arrow in the margin. The text glides up to it as the reader goes; the
  eye never hunts.
- **The tally.** Red means recording, and appears only then.
- **The rundown.** The shots in order, with what has played.
- **Timecode.** A take's running time.

## Tokens

| name | value | for |
|---|---|---|
| glass | `#000000` | the text field, the monitor |
| ink | `#f2f4f7` | text: 100% ahead on the reader's line, 55% on lines to come, 30% said |
| cue | `#ffb800` | the next word and the reading arrow, nothing else |
| tally | `#ff3b30` | recording |
| recorded | `#3ddc84` | a line with a take; the level meter |
| missing | `#f28b82` | a shot never captured |
| chrome | `#17181b`, raised `#202227`, lines `#2c2f36` | everything around the glass |

Type is **Atkinson Hyperlegible Next** (`apps/fonts`), everywhere. The Braille
Institute drew it so letters can be told apart at a distance and at a glance,
which is the whole job. The script is set at 48 px, weight 500, left-aligned
and ragged right; the chrome at 13–15 px. Timecode uses tabular figures; the
slashed zero is the typeface's own, so 0 is never read as O.

## Principles

1. **One thing moves.** The text eases to the reading line. Nothing else
   animates unless the reader did something (a count of three before a take,
   a toast when one is kept).
2. **Amber is the next word.** It appears in two places: the word and the
   arrow pointing at its line.
3. **Shots are marks, not words.** A small diamond in the text where a shot
   starts; its name is in the rundown and under the monitor, so the prose
   reads uninterrupted.
4. **State lives in the tally bar,** with the keys that matter now shown as
   keycaps, and nothing else.
5. **Words say what happens.** "Record", "Keep take", "Pause". Status is a
   sentence in sentence case: "Kept 2 lines: welcome, deploy".
