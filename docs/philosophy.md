# Journey Philosophy

## What Journey is

Journey is durable, file-based memory for engineering work. Agents operate it on the user's behalf.

Agent sessions are short-lived. Context windows fill up, get compacted, and end. Engineering work is not short-lived: it spans days or weeks, several sessions, several agents and several repos. Journey is where the context of that work survives between sessions. Any agent, at any time, can pick the work up from Journey and continue it.

The user directs and agents do the work. The user never needs to open a journey themselves. They say "start a journey about X" or "open a journey for this and record everything", and the agent handles the rest.

## Principles

### 1. Agents are the primary users

The CLI is the interface. A human-facing UI is not a goal.

- Every capability works non-interactively. Nothing is available only behind an interactive screen.
- Output is meant for machines first: stable, structured, and complete enough that an agent never has to guess or parse prose.
- Humans read journeys the way they read any files: in their editor, their terminal, or through an agent.

### 2. Memory, not a workspace

Journey holds context. It does not host the work.

The terminal, the editor and the agent are the workspace. Journey does not try to replace them, wrap them, or keep the user inside it. That rules out the TUI, an embedded terminal, terminal-session persistence, and a multiplexer sidebar.

### 3. Self-explaining

An agent with no prior instruction must be able to discover Journey and learn from the tool itself:

- **What** Journey is and **why** it exists, not only how to call it.
- **When** to create a journey or a side-quest, and when not to.
- **How** to find a journey's own conventions and follow them, rather than imposing a structure.
- **How** to resume an effort that someone else started.

The user should never have to point an agent at a guidance file. Guidance reaches agents in two ways:

- **In the CLI itself.** The binary carries the agent instructions and prints them on request, so they are always current with the installed version. Copies written into user folders go stale.
- **As agent skills.** Journey provides skills that the user installs globally for their agents. A skill is what lets an agent recognize "this should be a journey" or "this is a side-quest" and act on it, without the user explaining Journey first.

### 4. Unopinionated about what goes inside

What agents write is the point of Journey, but how it is organized is the user's call.

- Journey does not generate content and does not prescribe workflow, document structure, or where state is kept.
- Different journeys may be run in completely different ways: one with task files and a roadmap, another with a single running log, another with plans from a workflow tool. Journey supports all of them equally.
- Anything an agent records, including where the effort stands and what comes next, is legitimate content. This replaces the old rule against "handoff docs" and `NOW.md`. That rule was meant to stop Journey from generating state, not to stop users and agents from writing it.
- Journey does not track tasks, and it neither manages nor forbids task files.
- Journey relies on as few conventions as possible. The README is one of them: it is where a side-quest's outcome line lands in its linked journey.

### 5. One effort, one context

Context pollution is the enemy. Work that is unrelated to an effort should not end up in that effort's context, even when it was discovered while working on it. This is why side-quests exist.

### 6. Lifecycle matches the work

Work ends, and Journey's containers should end with it.

- Short work gets a short-lived container. Long work gets a long-lived one.
- Closing records an outcome. A container that ends silently loses the one thing worth keeping.
- What is closed leaves the default view but stays on disk.

### 7. Git owns code

- Journey never snapshots, stashes, restores or captures code state.
- Worktrees are referenced, not owned. The `worktrees/` symlinks are only a convenience view.
- A worktree belongs to one open container at a time.

### 8. Small surface

Journey stays small and quiet. It provides the container, the links between containers and the lifecycle. It stays out of what goes inside.

## Concepts

### Journey

A long-lived container for an engineering effort, such as an investigation, migration, feature or initiative. It holds:

- **Identity:** what the effort is and why it exists.
- **Overview:** the README.
- **Content:** whatever the user and agents keep there, organized however that journey is run.
- **Linked worktrees:** the code the effort works on.
- **Lifecycle:** active, paused, or closed.

### Side-quest

A small, short-lived container for work that comes up in the middle of the day. For example, you find a bug while working on something else and want to fix it quickly without derailing the main effort.

A side-quest is a kind of journey. It lives in the same store as journeys and is marked by its kind, which is why promoting it is just a change of kind.

- It has its own context. Its docs live and die with it.
- When it starts, the agent that creates it copies in the context from the linked journey that is relevant to the side-quest. It neither copies everything nor relies on a bare pointer.
- It may be linked to a journey, or to none.
- It can read context from a linked journey but never writes its work into that journey. It protects the parent's context instead of adding to it.
- If it touches code, it gets its own branch and worktree, separate from the journey's.
- It is never nested inside a journey. Side-quests and journeys sit side by side, and the relationship is a link, not a location.
- Closing it requires an outcome line. If it is linked to a journey, that line is also written into the journey's README, so the journey learns the result without absorbing the work.
- It can be promoted to a journey when it turns out bigger than expected, without losing its history or moving files.

### Links

Containers relate through links, not through folder structure. A link means "this came from" or "this belongs to". Linking, unlinking or promoting never moves files.

## How agents use Journey

There are three entry flows, and all of them start outside Journey:

1. **Start and explore.** "Start a journey about X." The agent creates the journey, records the goal, then works and records as it goes.
2. **Capture mid-session.** "We're deep in this, open a journey and record everything." The agent creates the journey and writes down what the session already knows, before it is lost to compaction or session end.
3. **Side-quest.** "Quickly look into this bug." The agent opens a side-quest, links it to the current journey if there is one, does the work, and closes it with an outcome.

One flow continues existing work:

4. **Resume.** "Continue the memory work." The agent finds the right container, learns how that journey is run, and continues.
