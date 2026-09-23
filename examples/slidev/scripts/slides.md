---
teleprompt: 1
scene:
  slides:
    adapter: slidev
    deck: examples/slidev/deck/slides.md
---

# Narrated slides

This video is a Slidev deck, narrated. Every sentence is a speaker note in the deck, spoken by Kokoro, and every picture is a slide from that same ordinary deck. {#this-video-is}

```teleprompt scene=slides policy=concurrent
1
```

# Your deck stays a deck

The deck does not change to be narrated. {#the-deck-does}

```teleprompt scene=slides policy=concurrent
2
```

You can still present it with Slidev. {#you-can-still}

```teleprompt scene=slides policy=concurrent
2?clicks=1
```

You can still export it. {#you-can-still-2}

```teleprompt scene=slides policy=concurrent
2?clicks=2
```

teleprompt only names the slides, and reveals each point as it is spoken. {#teleprompt-only-names}

```teleprompt scene=slides policy=concurrent
2?clicks=3
```

# A block names a slide

A block names one slide, and how many clicks into it, the way Slidev's own URLs do. You do not write these blocks yourself: teleprompt drafts them from the notes. {#a-block-names}

```teleprompt scene=slides policy=concurrent
3
```

# Where the time comes from

Each slide stays on screen for exactly as long as its sentence takes to say. The narration decides the timing, and the slides follow. {#each-slide-stays}

```teleprompt scene=slides policy=concurrent
4
```

# Edit a sentence. Rebuild.

The draft is yours to edit from here. Change a sentence and rebuild, and only what changed is spoken, or exported, again. {#the-draft-is}

```teleprompt scene=slides policy=concurrent
5
```

