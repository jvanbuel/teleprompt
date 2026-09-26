---
teleprompt: 1
---

# A tour

Welcome to Acme. Let me show you around. {#welcome}

```teleprompt scene=mock
wait 1500ms
```

Deployment is one command, and it streams progress as it goes. {#deploy}

```teleprompt scene=mock policy=concurrent cue="streams progress"
wait 2000ms
```
