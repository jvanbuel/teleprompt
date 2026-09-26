---
teleprompt: 1
scene:
  demo:
    adapter: mock
---

# A tour

Welcome to Acme. Let me show you around. {#welcome}

```teleprompt scene=demo
wait 1500ms
```

Deployment is one command, and it streams progress as it goes. {#deploy}

```teleprompt scene=demo policy=concurrent cue="streams progress"
wait 2000ms
```
