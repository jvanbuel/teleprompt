---
teleprompt: 1
locales:
  source: en
scene:
  mock:
    adapter: mock
output:
  transition: { duration: auto, max_ms: 600 }
---

# Introduction

Welcome to Acme. In the next two minutes I will show you how to get a project
running, deploy it, and roll it back when something goes wrong. {#welcome}

```teleprompt scene=mock
wait 400ms
```

Everything you see here is generated from a single Markdown file that lives in
version control alongside the code it documents. {#provenance}

```teleprompt scene=mock policy=concurrent align=start
wait 1200ms
mark
wait 600ms
```

# Deploying

Deployment is one command, and it streams progress as it goes. {#deploy}

```teleprompt scene=mock policy=fit-action
wait 2200ms
```

<!-- teleprompt: pause 600ms -->

If a deploy goes wrong, rolling back takes the same single command with one
extra flag. {#rollback}

```teleprompt scene=mock policy=trim-action
wait 3000ms
```
