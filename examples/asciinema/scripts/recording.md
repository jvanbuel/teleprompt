---
teleprompt: 1
locales:
  source: en
output:
  resolution: [1920, 1080]
  fps: 30
  transition: { duration: auto, max_ms: 300 }
---

# A session, recorded once

This terminal session was recorded once, with asciinema, and is never run
again. First, it checks a script, which costs nothing and changes nothing.
{#check}

```teleprompt scene=recording policy=stretch-action include=casts/tour.cast#check
```

Then it plans the script: one line per shot, with how long each sentence
takes to say. The recording's own markers split it, and each block names
the part it shows. {#plan}

```teleprompt scene=recording policy=stretch-action include=casts/tour.cast#plan
```

Last, the doctor lists what this build can capture, asciinema included. The
pauses in the recording stretch to fit each sentence, and the typing keeps
its pace. {#doctor}

```teleprompt scene=recording policy=stretch-action include=casts/tour.cast#doctor
```
