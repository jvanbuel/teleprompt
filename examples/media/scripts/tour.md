---
teleprompt: 1
locales:
  source: en
output:
  resolution: [1920, 1080]
  fps: 30
  transition: { duration: auto, max_ms: 400 }
---

# Media

Some pictures need no tool at all. A title card is a line of text on a
colour, drawn by ffmpeg. {#title}

```teleprompt scene=media policy=concurrent
title text="Media scenes" subtitle="images, clips and title cards"
```

An image is shown for exactly as long as its sentence takes. This one is a
frame of another teleprompt video, a Slidev deck revealing a list a point at
a time. {#image}

```teleprompt scene=media policy=concurrent
image src=slide.png
```

A clip plays the range it names, then holds its last frame for as long as
the sentence goes on. These four seconds come from a recorded terminal
session. {#clip}

```teleprompt scene=media policy=concurrent
clip src=terminal.mp4 from=0:01 to=0:05
```

Replace a file in the media folder, and the shots that show it are drawn
again. Nothing else is. {#rebuild}

```teleprompt scene=media policy=concurrent
title text="Replace a file. Rebuild."
```
