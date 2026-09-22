---
teleprompt: 1
locales:
  source: en
scene:
  ui:
    adapter: playwright
  terminal:
    adapter: vhs
output:
  resolution: [1280, 720]
  fps: 24
---

# Reading an Airflow log

## In the browser

Airflow's web interface starts, like most of them, with a sign-in.
{#signin}

```teleprompt scene=ui policy=concurrent
await page.goto('http://localhost:8080/');
await page.fill('#username', 'airflow');
await page.fill('#password', 'airflow');
await page.click('input[type="submit"]');
await page.waitForLoadState('networkidle');
```

From there, one task's log is a search and three clicks: narrow the DAG
list, open the DAG, pick the task out of the grid, and switch to its log
tab. Each click waits on the server, and what you get at the end is seven
lines of output. {#clicking}

```teleprompt scene=ui policy=concurrent
await page.fill('input[placeholder*="Search"]', 'example_bash_operator');
await page.click('a:has-text("example_bash_operator")');
await page.waitForLoadState('networkidle');
await page.locator('[data-testid="task-instance"]').first().click();
await page.getByRole('tab', { name: /Logs/i }).click();
await page.waitForTimeout(2500);
```

## In the terminal

Flowrs reads the same REST API from a terminal. One command, no sign-in,
and the DAG list is already on screen. {#flowrs}

```teleprompt scene=terminal policy=concurrent
Set TypingSpeed 40ms
Type "flowrs run"
Enter
Sleep 5s
```

Filtering is a slash and as much of the name as you can remember.
{#filter}

```teleprompt scene=terminal policy=concurrent
Type "/"
Sleep 500ms
Type "example_bash_operator"
Sleep 2s
Enter
Sleep 1s
```

Enter walks down the hierarchy — the DAG, its runs, the tasks in a run,
and finally the log. Escape walks back up. Same answer, no page loads, and
nothing to click. {#drill}

```teleprompt scene=terminal policy=concurrent
Down
Sleep 1s
Enter
Sleep 2s
Down
Sleep 1s
Enter
Sleep 2s
Down
Sleep 1s
Enter
Sleep 3s
```
