---
teleprompt: 1
locales:
  source: en
output:
  resolution: [1280, 720]
  fps: 30
timing:
  min_stretch: 0.2
  max_stretch: 30
---

# Reading with Teleprompt

## The prompter

Teleprompt turns a script into a narrated video, and its Linux app is the
prompter you read it from. Open a script, and read it aloud. {#intro}

```teleprompt scene=welcome policy=fit-action
Sleep 1s
Move@900ms 197 485
Sleep 1s
```

A script opens on the glass. The glass is black, because a beam-splitter
reflects nothing black, and the next word to read is in amber. {#glass}

```teleprompt scene=app policy=fit-action
Sleep 2s
```

## Making it readable

Plus and minus change the size of the text, for wherever the screen
stands. {#size}

```teleprompt scene=app policy=fit-action
Sleep 500ms
Key + 3
Sleep 1500ms
Key - 3
Sleep 500ms
```

M mirrors it, for a rig that reflects the screen into a sheet of glass in
front of the lens. {#mirror}

```teleprompt scene=app policy=fit-action
Sleep 300ms
Key m
Sleep 2s
Key m
Sleep 300ms
```

## Recording a take

To record from the line you're on, press Control, Shift and Space. A
countdown gives you time to look up. {#record}

```teleprompt scene=app policy=fit-action
Sleep 300ms
Ctrl+Shift+Space
Sleep 3s
```

Then the script follows your voice, word by word, and a red frame around
the glass says you're on air. {#follow}

```teleprompt scene=app policy=fit-action
Sleep 5s
```

Press the same keys again to keep the take. Every line you read to its end
is kept, and Undo puts back what was there before. {#keep}

```teleprompt scene=app policy=fit-action
Sleep 1500ms
Ctrl+Shift+Space
Sleep 3s
```

## Finding your way

Every action has a key, and the question mark lists them all. {#keys}

```teleprompt scene=app policy=fit-action
Sleep 300ms
Key ?
Sleep 3s
Escape
Sleep 300ms
```

When you build the video, what you recorded replaces the synthesized voice,
line by line. {#outro}

```teleprompt scene=app policy=fit-action
Sleep 2s
```
