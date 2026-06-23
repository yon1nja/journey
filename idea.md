Journey

Journey is a context persistence system for AI-assisted engineering.

It provides a durable workspace for a single engineering effort, allowing developers and AI agents to maintain context across interruptions, priority changes, multiple repositories, and long-running work.

Unlike task managers, Journey does not focus on tracking work items.

Unlike note-taking tools, Journey does not focus on knowledge management.

Journey focuses on preserving and restoring working context.

⸻

Core Concept

A Journey represents a single engineering effort:

Examples:

Investigate production authentication failures
Design EDL package publishing
Implement new editor architecture
Review PR-1234
Migrate service to a new API

Each Journey owns everything required to continue that effort later:

Notes
Decisions
Questions
Draft docs and specs
Worktree links
Repository state
Current progress
Next actions

The goal is that after hours, days, or weeks away from a problem, a developer can return and immediately resume productive work.

⸻

Philosophy

Engineering work is frequently interrupted.

Developers:

* switch priorities
* work across multiple repositories
* spawn multiple AI conversations
* investigate issues over several days
* revisit architectural decisions months later

Most tools preserve artifacts:

Git preserves code
Jira preserves tickets
Slack preserves conversations

But very few tools preserve:

“What exactly was I doing and what should I do next?”

Journey exists to preserve that context.

⸻

Relationship with AI Agents

Journey does not manage AI agents.

Instead, Journey provides a structured environment that AI agents can operate within.

A companion Journey Skill instructs agents to:

* understand the current Journey
* maintain context documents
* record decisions
* track discoveries
* update next actions
* prepare handoffs

The Journey stores state.

The AI agent maintains it.

⸻

Structure

A Journey contains:

Journey
├── Metadata
├── Generated Context
├── Journey Docs
└── Worktree Links

Example:

error-investigation/
├── journey.yaml
├── journal.jsonl
├── NOW.md
├── docs/
│   ├── investigation.md
│   └── migration-plan.md
└── worktrees/
    ├── frontend -> /actual/path/to/frontend-worktree
    └── backend -> /actual/path/to/backend-worktree

A Journey may reference live worktrees from multiple repositories when solving a problem requires changes across system boundaries. The `worktrees/` directory is a generated convenience index of symlinks; Journey does not own the checkouts.

A Journey also owns effort-local docs and specs. These are human-authored working files that do not belong at a repository root until the developer intentionally decides to publish or commit them somewhere.

⸻

Design Principles

1. Context is the primary artifact

Journey optimizes for preserving context, not managing tasks.

2. Work is organized around efforts

A Journey represents a work effort rather than a ticket, branch, or repository.

3. AI-first

Journey assumes AI agents are active participants in the workflow and should be able to maintain context automatically.

4. Minimal and local

Journey stores its state as files and folders.

No server is required.

No external service is required.

The initial implementation is a Rust CLI that stores local files and shells out to git for repository state.

5. Recoverability

A Journey should always answer:

What is this?
What has been learned?
What decisions were made?
What remains unresolved?
What should happen next?

⸻

Vision

The long-term vision of Journey is to become the missing layer between:

Git
AI Agents
Terminal Workflows
Engineering Knowledge

allowing developers to pause and resume complex engineering efforts with minimal loss of context.

In a world where engineers increasingly collaborate with AI agents, Journey serves as the persistent memory of the work itself.
