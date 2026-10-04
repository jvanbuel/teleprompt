---
theme: default
colorSchema: dark
title: A voice that moos
layout: cover
canvasWidth: 860
---

# A voice that moos

A speech server for teleprompt: a hundred lines of Python, and a real cow

---

# The API is the plugin

teleprompt sends each line as OpenAI's speech request:

```http
POST /v1/audio/speech
{"model": "moo-2", "input": "Hello.", "voice": "cow",
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

# The request, typed

<<< @/snippets/moo_server.py#request

FastAPI reads each request into this, and refuses one that does not fit.

---

# Answering it

<<< @/snippets/moo_server.py#answer {1-2|3-4|5-7|10-12}

---

# A moo for every word

<<< @/snippets/moo_server.py#speak {4-7|5,7}

---

# Making a moo

<<< @/snippets/moo_server.py#moo {8-13|1-5}

---

# Voices are pitches

<<< @/snippets/moo_server.py#voices

One recording of a real cow, CC0: `moo.wav`, beside the server.

---

# Naming it

```toml {1-3|5-7}
[backends.moo]
base_url = "http://localhost:8890/v1"
model = "moo-2"

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
