---
teleprompt: 1
output:
  resolution: [1920, 1080]
  fps: 30
  transition: { duration: auto, max_ms: 300 }
---

# A voice that moos

This is a speech server that says every word as a moo. It is about a
hundred lines of Python, and teleprompt needs no plugin to use it. {#intro}

```teleprompt scene=slidev policy=concurrent
1
```

**Cow:** Hello. I am the cow, and I approve of this message. {#hello}

# The API is the plugin

Teleprompt speaks OpenAI's speech API. For each line, it sends one
request: the text, a voice, a speed, and the format it wants back. {#api}

```teleprompt scene=slidev policy=concurrent
2
```

It wants raw samples, sixteen bits at twenty four kilohertz. Anything that
answers this request is a voice. {#pcm}

```teleprompt scene=slidev policy=concurrent
2?clicks=1
```

# Answering the request

The server answers one path, the speech request. {#path}

```teleprompt scene=slidev policy=concurrent
3
```

It reads the JSON body, and refuses a voice it does not have by name, so
the error reaches you in words. {#refuse}

```teleprompt scene=slidev policy=concurrent
3?clicks=1
```

Then it speaks the input, and sends the samples back. {#send}

```teleprompt scene=slidev policy=concurrent
3?clicks=2
```

# A moo for every word

Every word becomes a moo, as long as the word. Punctuation becomes a
pause, so a sentence still sounds like a sentence. {#words}

```teleprompt scene=slidev policy=concurrent
4
```

The speed teleprompt sends stretches both. {#speed}

```teleprompt scene=slidev policy=concurrent
4?clicks=1
```

# Making a moo

A moo is a hum that opens into an oo, and sinks in pitch as it fades. A
little wobble keeps it alive. {#hum}

```teleprompt scene=slidev policy=concurrent
5
```

Each voice is just a pitch to start from, and one to sink to. {#pitches}

```teleprompt scene=slidev policy=concurrent
6
```

**Calf:** Like this, up high. {#calf}

**Bull:** Or down here. {#bull}

# Being a good citizen

Two more paths make it easy to work with. Teleprompt checks a script's
voices against the list before it speaks a word, and setup asks for the
models, to see that the server is up. {#list}

```teleprompt scene=slidev policy=concurrent
7
```

# Naming it

In teleprompt dot toml, the server gets a name, the API it speaks, and
its address. {#name}

```teleprompt scene=slidev policy=concurrent
8
```

Then a speaker in the cast can use it, and a line that starts with their
name is theirs. {#cast}

```teleprompt scene=slidev policy=concurrent
8?clicks=1
```

**Cow:** Moo is my whole vocabulary, and I am at peace with that. {#peace}

# Yours

The captions keep the words, and the cache keeps every moo, so a rebuild
asks for nothing twice. Swap the moo for your own model, and everything
else stays the same. {#yours}

```teleprompt scene=slidev policy=concurrent
9
```

The server, these slides, and this script are all in the examples
folder, under moo. {#where}
