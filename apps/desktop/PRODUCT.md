# Suneiro

<!-- impeccable:product-schema 1 -->

## Platform

web

React UI inside a Tauri desktop app; macOS first.

## Product Purpose

Chat with Claude Code, Codex and OpenCode, keep related tasks in connected conversations, review their changes and ship pull requests from one desktop app. Separate workspaces provide separate git checkouts; conversations within a workspace share files.

## Users and Operating Context

Developers working across coding tasks, agent conversations, terminals and GitHub pull requests. Agents use their existing CLI authentication; the app does not store or proxy credentials.

## Capabilities and Constraints

Chat is the main interface. Keep ordinary model selection, parent/child/sibling conversations, agent questions the user can fill out, compact updates from related tasks, human diff review, PRs, terminals, checkpoints and keyboard shortcuts. Use sensible defaults rather than setup forms. Agent profiles, context-handoff builders, custom reviewers and advanced workflow consoles are outside the product surface. Support system, light and dark themes. Keep existing sessions, scripts, saved settings and workspace paths usable.

Provider capabilities differ and account limits still apply. Questions require an explicit user answer. Plan mode is not an OS sandbox. Preserve runtime safety and restrictions on existing sessions even when their old setup controls are not part of the interface. Isolated delegated work starts from a committed revision; uncommitted parent changes are not silently copied.

## Brand Commitments

The user selected Suneiro and approved wiring it into the product. The selected direction is an intertwined S mark with ink blue and quiet, precise typography. Light mode uses white with neutral gray supporting surfaces. Domain registration and a hosted website are not established by this repository.
