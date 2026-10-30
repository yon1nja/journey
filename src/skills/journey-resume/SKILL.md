---
name: journey-resume
description: Resume work recorded in a Journey when the user asks to continue, pick up, or get back to an effort, or refers to a journey by name. Examples - "continue the memory work", "where were we on the auth failures", "pick up journey X".
---

# Resume a Journey

1. Run `journey guide` and read it. It is the authoritative guide for the installed version.
2. Find the journey: `journey list --search "<words>"`, or `journey list` for the active ones. If several match, ask the user which one.
3. Get its summary: `journey status <id> --json`. It gives the folder path, README, docs, linked repos with branches, and related journeys.
4. Learn how this journey is run before acting: read its `README.md`, `AGENTS.md`, and the docs it points to. Follow its conventions.
5. Continue the work in the linked worktrees, and keep the journey's record current as you go.
