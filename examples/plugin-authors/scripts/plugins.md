---
teleprompt: 1
locales:
  source: en
output:
  resolution: [1920, 1080]
  fps: 30
  transition: { duration: auto, max_ms: 300 }
---

# Writing a plugin

A scene plugin brings a tool your script can show. It wraps the tool, and
records what your script asks of it. This blue card was drawn by one:
about a hundred lines of Python, in the examples folder. {#card}

```teleprompt scene=card policy=concurrent
color #1d4ed8
```

# A program with a name

A plugin is a program whose name says what it is: teleprompt scene, then
a name. Put it on your path, and teleprompt plugins lists it, with the
tools it needs. {#name}

```teleprompt scene=asciinema policy=fit-action include=casts/session.cast#find
```

# Talking to it

Teleprompt starts it when a command needs it, and talks to it in JSON,
one line at a time. Send it a block, and it answers with the shots, and
how long each one takes. {#ask}

```teleprompt scene=asciinema policy=fit-action include=casts/session.cast#ask
```

Those answers never start your tool, so check and plan stay fast and
offline. When a line is wrong, the error points at the script, in your
plugin's own words. {#check}

```teleprompt scene=asciinema policy=fit-action include=casts/session.cast#check
```

Build asks it to capture each shot, and it writes a clip for each. Here
are two of its cards, a second each. {#capture}

```teleprompt scene=card policy=concurrent
color #f59e0b
hold 1s
mark
color #10b981
hold 1s
```

# Voices are servers

A voice is not a plugin. It is a speech server that speaks OpenAI's API,
written in any language: give it a name and an address. I am one, called
studio. {#studio}

```teleprompt scene=asciinema policy=fit-action include=casts/session.cast#studio
```

The examples folder has a small one that speaks with eSpeak, in Python's
standard library. It is given each line, and answers with its sound.
{#voice}

```teleprompt scene=card policy=concurrent
color #7c3aed
```

**eSpeak:** Like this. I am the eSpeak server, and I run offline. {#espeak}

Write yours in any language you like. The guides are in the docs, one
for each, and both examples are ready to copy. {#yours}

```teleprompt scene=card policy=concurrent
color #111827
```
