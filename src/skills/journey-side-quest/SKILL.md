---
name: journey-side-quest
description: Open a Journey side-quest when the user wants to handle something small that came up during other work, such as a bug found along the way or a quick investigation, without polluting the current journey's context. Examples - "side-quest this", "open a side-quest for this bug", "quickly fix this on the side". Also use it to close such a side-quest with its outcome.
---

# Journey side-quest

A side-quest is a small, short-lived kind of journey with its own context. It may be related to a journey or stand alone.

1. Run `journey guide` and read its side-quest section. It is the authoritative guide for the installed version.
2. Find the journey this came from, if any: `journey status --json` from the current folder, or `journey list --search "<words>"`.
3. Create it: `journey new "<title>" --side-quest --linked-to <journey-id> --description "<what and why>" --json`. Leave out `--linked-to` if it belongs to no journey.
4. Read the linked journey and copy into the side-quest only the context this side-quest needs. Don't copy everything, and don't write the side-quest's work into the linked journey.
5. If it touches code, give it its own branch and worktree: `journey worktree new --branch <name> --repo <path> --journey <side-quest-id>`.
6. When it's finished: `journey close <side-quest-id> --outcome "<result>"`. The outcome is also added to the linked journey's README.
7. If it turns out bigger than a side-quest, ask the user, then `journey promote <side-quest-id>`.
