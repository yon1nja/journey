---
name: journey-start
description: Start a Journey, durable file-based memory for an engineering effort, when the user asks to start, open, or create a journey about a topic, or asks to record the current session's work so it survives beyond this session. Examples - "start a journey about X", "open a journey for this and record everything".
---

# Start a Journey

Journey keeps an effort's context on disk so any later agent session can continue it. The `journey` CLI manages it.

1. Run `journey guide` and read it. It is the authoritative guide for the installed version.
2. Check whether a journey already exists for this: `journey list --search "<words>"`. If one does, confirm with the user before creating another.
3. Create it: `journey new "<title>" --description "<what this effort is and why>" --json`. The JSON gives the id and folder path.
4. Link the code it works on: `journey link <path> --journey <id>`, or `journey worktree new --branch <name> --repo <path> --journey <id>` for a fresh branch.
5. Record the context in the journey folder. If you are opening the journey mid-session, write down now everything this session already knows: the goal, what was tried, findings, decisions, current state, next steps.
6. Keep recording as the work goes on, following the guide.
