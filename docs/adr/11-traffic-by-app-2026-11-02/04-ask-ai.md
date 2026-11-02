# 4. "What is this?": ask Claude or ChatGPT by a link

**Status:** Proposed (not built).

## Context

The user asked (2026-11-02): "it would be great to ask a chat, not inside
LocalRouter, but by a deep link or a link that sends it to ChatGPT".
LocalRouter has no chat and does not get one.

## Decision

The details window has **Ask Claude**, **Ask ChatGPT** and **Copy question**.
A button opens the default browser (or the desktop app, if it handles the
link) with a new chat and the question filled in.

### 4.1 The links

| Button | URL |
|---|---|
| Ask Claude | `https://claude.ai/new?q=<question>` |
| Ask ChatGPT | `https://chatgpt.com/?q=<question>` |

Both forms are **not verified yet**. Task 0 opens each with a test question
and records what happens. If a service ignores `q`, its button copies the
question to the clipboard, opens the base URL and shows "Question copied:
paste it with ⌘V".

### 4.2 The question

`AskAI.question(for:)` in `LocalRouterKit/AskAI.swift` is one pure function.
For the app in the canvas it returns:

```
What is the macOS program "helperd"? It runs from
~/Library/Application Support/.helperd/helperd and starts as LaunchAgent
com.helperd.agent. It has no code signature. SHA-256 9f2c…a8e41a.
VirusTotal: 3 of 72 engines, "Trojan.MacOS.Agent". It sent 4.1 MB to
203.0.113.45 (Netherlands). Is it safe, and how do I remove it?
```

Rules:

1. The home folder is always written as `~`. The user name never appears.
2. Network addresses are included only when **Put network addresses in the
   question** is on (Settings → Traffic, on by default).
3. The whole URL stays under 2,000 characters: the function drops the address
   list first, then shortens the hash.
4. The question is in English. The chat answers in the user's language if the
   user asks it to.

The question is shown in the window before any button is pressed, so the
user sees what leaves the Mac.

## Tests

T12 in the [test plan](09-test-plan.md), and M6.
