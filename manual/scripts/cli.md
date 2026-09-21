---
teleprompt: 1
locales:
  source: en
scene:
  terminal:
    adapter: vhs
output:
  resolution: [1920, 1080]
  fps: 30
  transition: { duration: auto, max_ms: 600 }
---

# The manual is the program

This is teleprompt's command-line manual, and it is compiled by the tool it
documents. Every sentence you are hearing is a paragraph of Markdown in one
file, and every command you watch run is a tape in that same file, a few
lines below the sentence about it. {#welcome}

```teleprompt scene=terminal
Set TypingSpeed 35ms
Type "teleprompt --help"
Enter
Sleep 2s
```

That is the whole idea. Prose is narration, fenced blocks are what the
screen does, and the length of the speech decides how long the picture
holds. Rewrite this paragraph and the terminal below it waits longer,
because it is waiting for me. {#prose-and-action}

```teleprompt scene=terminal policy=concurrent
Set TypingSpeed 35ms
Type "teleprompt check manual/scripts/cli.md"
Enter
Sleep 1s
```

The terminal you are looking at is a VHS tape — Charm's recording format,
borrowed whole. teleprompt reads the tape language rather than shelling out
to VHS, because VHS renders a tape to one finished file and cannot pause in
the middle of it, and pausing in the middle is exactly where narration
goes. {#vhs}

```teleprompt scene=terminal
# The tape teleprompt is reading to draw this very shot.
Set TypingSpeed 60ms
Type "Set TypingSpeed 60ms"
Enter
Type "Type \"teleprompt plan\""
Enter
Sleep 1500ms
```

Where one paragraph hands over to the next, a tape carries a mark, and the
mark is spelled as a comment. VHS ignores it, so the file stays a tape VHS
itself will run — which is the whole reason to keep a demo in a real tape
file rather than in a dialect only teleprompt reads. {#the-mark}

```teleprompt scene=terminal policy=concurrent
Set TypingSpeed 60ms
Type "# mark"
Enter
Sleep 1200ms
```

# Starting a project

`teleprompt new` scaffolds one: a configuration file, a scripts directory,
and a timelines directory for the compiled output you commit alongside the
prose. {#new}

```teleprompt scene=terminal
Set TypingSpeed 35ms
Type "teleprompt new demo"
Enter
Sleep 900ms
# mark
Type "ls demo"
Enter
Sleep 1200ms
```

The demo script it leaves behind is two paragraphs and two action blocks —
small enough to read in one breath, and complete enough to compile. Nothing
else has to be installed for it to work. {#scaffold}

```teleprompt scene=terminal policy=concurrent
Set TypingSpeed 25ms
Type "cat demo/scripts/demo.md"
Enter
Sleep 2s
```

# Reading a script

`teleprompt check` parses and validates, and does nothing else. No
synthesis, no network, no files written — so it costs nothing to run it on
every keystroke, and plenty of editors will. {#check}

```teleprompt scene=terminal
Set TypingSpeed 30ms
Type "teleprompt check demo/scripts/demo.md"
Enter
Sleep 800ms
```

When it does have something to say, it says where. A mistyped policy, a
duration that is not a duration, a tape directive teleprompt cannot run:
each is reported at the line that carries it, with the spelling it should
have had. {#diagnostics}

```teleprompt scene=terminal policy=stretch-action
# One span, no `Mark`: the whole exchange is paced against the sentence
# above it, which is what `stretch-action` is for.
Set TypingSpeed 60ms
Type "teleprompt check demo/scripts/broken.md"
Enter
Sleep 3s
Type "teleprompt check demo/scripts/broken.md --format json"
Enter
Sleep 4s
```

# Compiling the timeline

`teleprompt plan` compiles the script into a timeline and prints one line
per beat: when it starts, which pacing policy governs it, and how long its
narration runs. {#plan}

```teleprompt scene=terminal
Set TypingSpeed 30ms
Type "teleprompt plan demo/scripts/demo.md"
Enter
Sleep 2500ms
```

Those durations come from the cache when a segment has been synthesized
before, and from a word-count estimate when it has not. Plan never
synthesizes anything, which is why it answers instantly on a laptop with no
voice backend anywhere near it. {#estimated-vs-measured}

```teleprompt scene=terminal policy=concurrent
Set TypingSpeed 25ms
Type "teleprompt plan demo/scripts/demo.md --format json | grep duration_source"
Enter
Sleep 1800ms
```

An action block's duration is different: a tape states its own timing. Every
sleep is written down and every keystroke has a speed, so teleprompt adds
them up rather than guessing, and never has to run the terminal to find out
how long the terminal takes. {#exact-actions}

```teleprompt scene=terminal
Set TypingSpeed 30ms
Type "teleprompt plan demo/scripts/demo.md --format json | grep -A2 action"
Enter
Sleep 2s
```

# The feedback loop

Commit the timeline that `plan` produced, and `teleprompt diff` will compare
the next compile against it. This is the loop the whole tool is built
around. {#commit-the-timeline}

```teleprompt scene=terminal policy=concurrent
Set TypingSpeed 25ms
Type "teleprompt plan demo/scripts/demo.md --format json > demo/timelines/demo.en.json"
Enter
Sleep 600ms
```

Now edit a paragraph. Not the code, not the tape — the prose. A longer
sentence takes longer to say, and everything scheduled after it moves. {#edit-the-prose}

```teleprompt scene=terminal policy=stretch-action
# Typing speed is the knob `stretch-action` turns: teleprompt re-paces
# these keystrokes to fill the sentence rather than cutting it short.
Set TypingSpeed 80ms
Type "sed -i 's/whole point/whole point of the tool/' demo/scripts/demo.md"
Enter
Sleep 1s
```

And `diff` tells you exactly how much, before anything is rendered: which
beats moved, by how long, and what the video now runs to end to end. That
report is the review surface. It is what a pull request argues about instead
of arguing about a video file nobody can read. {#the-diff}

```teleprompt scene=terminal
Set TypingSpeed 30ms
Type "teleprompt diff demo/scripts/demo.md"
Enter
Sleep 3s
```

# Narration

`teleprompt dub` is the command that actually speaks. It synthesizes every
segment, writes the audio, and publishes a manifest: one entry per segment,
with its prose, its offset, its length, and the voice that produced
it. {#dub}

```teleprompt scene=terminal policy=concurrent
Set TypingSpeed 30ms
Type "teleprompt dub demo/scripts/demo.md --out public/narration"
Enter
Sleep 3s
# mark
Type "ls public/narration/en"
Enter
Sleep 1200ms
```

teleprompt renders no video. Something else owns the picture — Remotion, a
web player, your own compositor — and it reads the manifest. Deriving its
length from the manifest is what keeps the central property alive on the
other side of that boundary: narration still drives the pacing. {#manifest}

```teleprompt scene=terminal
Set TypingSpeed 25ms
Type "cat public/narration/en/narration.json"
Enter
Sleep 2500ms
```

Having spoken the script once, teleprompt has also measured it. The same
`plan` is still instant, and now every duration in it is a recording rather
than an estimate. {#measured}

```teleprompt scene=terminal policy=concurrent
Set TypingSpeed 25ms
Type "teleprompt plan demo/scripts/demo.md --format json | grep duration_source"
Enter
Sleep 1500ms
```

# Keeping a pull request honest

`dub --check` compares the committed manifest against the script and exits
three when they disagree. Put that in continuous integration and a change
to the prose can no longer be merged with last week's narration still
attached to it. {#dub-check}

```teleprompt scene=terminal policy=trim-action
Set TypingSpeed 30ms
Type "teleprompt dub demo/scripts/demo.md --out public/narration --check"
Enter
Sleep 2s
# mark
Type "echo $?"
Enter
Sleep 1500ms
```

<!-- teleprompt: pause 500ms -->

# When something is wrong

`teleprompt doctor` reports the environment teleprompt can see: which scene
adapters this build ships, which voice backends it knows, how warm the cache
is, and whether the server your project points at is answering. {#doctor}

```teleprompt scene=terminal
Set TypingSpeed 35ms
Type "teleprompt doctor"
Enter
Sleep 3s
```

A voice backend that is down is a warning there, not a failure. `check`,
`plan`, and `diff` never needed it; only `dub` does. That asymmetry is the
point: the loop you run a hundred times a day stays offline and instant, and
the one command that needs a machine to speak is the only one that asks for
it. {#offline-by-default}

```teleprompt scene=terminal policy=concurrent
Set TypingSpeed 30ms
Type "teleprompt doctor --format json | grep -A3 voice_probe"
Enter
Sleep 2s
```

This manual compiles in the repository that builds the tool, and CI compares
it against its committed timeline on every change. When a command grows a
flag, the sentence about it and the tape demonstrating it are in the same
file, in version control, and the build says so when they fall out of
step. {#self-hosting}

```teleprompt scene=terminal policy=concurrent
Set TypingSpeed 30ms
Type "teleprompt diff manual/scripts/cli.md"
Enter
Sleep 2s
```
