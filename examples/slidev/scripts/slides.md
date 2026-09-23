---
teleprompt: 1
locales:
  source: en
output:
  resolution: [1920, 1080]
  fps: 30
  transition: { duration: auto, max_ms: 400 }
---

# Narrated slides

This video is a Slidev deck, narrated. Every sentence is a paragraph of one
Markdown file, spoken by Kokoro, and every picture is a slide from an
ordinary deck. {#welcome}

```teleprompt scene=slides policy=concurrent
1
```

The deck does not change to be narrated. You can still present it with
Slidev. {#present}

```teleprompt scene=slides policy=concurrent
2?clicks=1
```

You can still export it. {#export}

```teleprompt scene=slides policy=concurrent
2?clicks=2
```

teleprompt only names the slides, and reveals each point as it is spoken.
{#names}

```teleprompt scene=slides policy=concurrent
2?clicks=3
```

A block names one slide, and how many clicks into it, the way Slidev's own
URLs do. {#block}

```teleprompt scene=slides policy=concurrent
3
```

Each slide stays on screen for exactly as long as its sentence takes to
say. The narration decides the timing, and the slides follow. {#timing}

```teleprompt scene=slides policy=concurrent
4
```

Edit a sentence and rebuild. Only what changed is spoken, or exported,
again. {#rebuild}

```teleprompt scene=slides policy=concurrent
5
```
