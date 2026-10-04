---
theme: default
colorSchema: dark
title: A voice that moos
layout: cover
canvasWidth: 860
---

# A voice that moos

A speech server for teleprompt, in about a hundred lines of Python

---

# The API is the plugin

teleprompt sends each line as OpenAI's speech request:

```http
POST /v1/audio/speech
{"model": "moo-1", "input": "Hello.", "voice": "cow",
 "speed": 1.0, "response_format": "pcm"}
```

<v-click>

and wants back raw samples: 16-bit, mono, 24 kHz.

```http
200 OK
Content-Type: audio/pcm
```

</v-click>

---

# Answering the request

<<< @/snippets/moo_server.py#request {1-3|4-9|10-15}

---

# A moo for every word

<<< @/snippets/moo_server.py#speak {5-9|6,9}

---

# Making a moo

<<< @/snippets/moo_server.py#moo

---

# Voices are pitches

<<< @/snippets/moo_server.py#voices

---

# Being a good citizen

<<< @/snippets/moo_server.py#list

`dub` checks a script's voices against the list. `setup` asks for the models.

---

# Naming it

```toml {1-4|6-8}
[backends.moo]
api = "openai"
base_url = "http://localhost:8890/v1"
model = "moo-1"

[voices.cow]
backend = "moo"
voice = "cow"
```

```md
**Cow:** Hello. I am the cow, and I approve of this message.
```

---
layout: cover
---

# Yours

Swap the moo for your model. The rest stays the same.
