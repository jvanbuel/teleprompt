---
teleprompt: 1
locales:
  source: en
output:
  resolution: [1920, 1080]
  fps: 30
  transition: { duration: auto, max_ms: 400 }
---

# Motion scenes

This video was compiled from one Markdown file. Every sentence you hear is a
paragraph of it, spoken by Kokoro, and every picture you see is a composition
from an ordinary Remotion project. {#welcome}

```teleprompt scene=motion policy=concurrent
Title {"title": "teleprompt × Remotion", "subtitle": "narrated video, compiled from Markdown"}
```

The words decide the timing. teleprompt measures how long each paragraph
takes to say, and tells Remotion to render the picture beneath it for
exactly that long. {#timing}

```teleprompt scene=motion policy=concurrent
Pipeline {"steps": ["Markdown", "Kokoro", "Timeline", "Remotion", "Video"]}
```

A Remotion block names a composition your project already registers, and
the props to render it with. Each paragraph gets a block of its own. {#blocks}

```teleprompt scene=motion policy=concurrent
Code {"lines": [
  "```teleprompt scene=motion",
  "Title {\"title\": \"Hello\"}",
  "```"
]}
```

So a short sentence gets a short shot. {#short}

```teleprompt scene=motion policy=concurrent
Caption {"text": "A short sentence, a short shot."}
```

And a long one, which takes its time to wander through a few extra clauses
before it reaches the point, gets a long one, and the animation stretches to
fill it, because every component is written against the length it was given
rather than a fixed number of frames. {#long}

```teleprompt scene=motion policy=concurrent
Durations {"rows": [
  {"label": "the short one", "ms": 2300},
  {"label": "this one", "ms": "slot"}
]}
```

Reword a paragraph and only the shot beneath it renders again. Everything
else comes out of the cache. {#cache}

```teleprompt scene=motion policy=concurrent
Title {"title": "Edit prose. Rebuild.", "subtitle": "teleprompt build scripts/remotion.md"}
```
