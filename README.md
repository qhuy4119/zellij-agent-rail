# zellij-agent-rail

A persistent sidebar rail for [Zellij](https://zellij.dev) 0.46+ that shows,
at a glance, which pane — in which tab, in which session — is working, done,
failed, or waiting for you. Click a row to jump there, switching sessions if
needed.

Unlike `multiplexer-sidebar` (which polls `list-sessions`/`list-panes`/`ps`
every 2–5 seconds as an external process), this is a **native Zellij wasm
plugin driven entirely by push events**:

| Data | Source |
|---|---|
| Pane/tab state | `TabUpdate`, `PaneUpdate` events |
| Agent status (working / blocked / done / failed) | `PaneAttentionChanged`, `GlobalAttentionUpdate` events — Zellij's own attention system, fed by the **OSC 7501** protocol that Claude Code (≥ 2.1.295), Pi (≥ 1.1.0), Codex, Crush, OpenCode 2.1+ and others emit natively |
| Other sessions + their counts | `GlobalAttentionUpdate` (cross-session, machine-wide) |
| Jumping | `FocusTerminalPane` / `FocusSessionPane` / `SwitchSession` |
| Marking seen | `MarkSessionPaneSeen` |

Zero polling, zero subprocesses, zero blocking host calls on any event path.
One wasm runs on Linux and Windows alike.

## Build

Requires Rust with the `wasm32-wasip1` target:

```sh
rustup target add wasm32-wasip1   # not needed on Arch system rust: it ships the target
cargo build --release --target wasm32-wasip1
# → target/wasm32-wasip1/release/zellij-agent-rail.wasm
```

> Note: Zellij plugins are **command-style wasm** (a binary crate exporting a
> WASI `_start` entry). Do not add `[lib] crate-type = ["cdylib"]` — that
> produces a reactor module (`_initialize`) that Zellij cannot load.

## Install

```sh
mkdir -p ~/.config/zellij/plugins
cp target/wasm32-wasip1/release/zellij-agent-rail.wasm \
   ~/.config/zellij/plugins/agent-rail.wasm
cp layouts/agent-rail.kdl ~/.config/zellij/layouts/
```

Windows (PowerShell): same paths under `%USERPROFILE%\.config\zellij\`.
The wasm artifact is identical on both platforms — build once, copy the same
file.

## Use

```sh
zellij --layout agent-rail
```

Or fold the `new_tab_template` block from `layouts/agent-rail.kdl` into
your `layouts/default.kdl` so every tab gets the rail.

On first load, focus the rail pane and approve the two permissions it
requests (`ReadApplicationState`, `ChangeApplicationState`). They are
recorded per absolute wasm path in `~/.cache/zellij/permissions.kdl`.

| Glyph | Meaning |
|---|---|
| `◉` | needs you (permission prompt, question, notification, bell) |
| `✗` | failed (non-zero exit / error) |
| `✓` | done |
| `▸` | working |
| `✚` | new output while you were elsewhere |
| `◌` | gone quiet |
| `·` | idle |

Mouse: click a row to jump (tab row → tab, pane row → pane, session row →
session). Scroll over Sessions or Agents to move that list. Jumping marks the pane seen.

## Agents

No hooks, no wrappers, no setup: Claude Code ≥ 2.1.295 and Pi ≥ 1.1.0 report
`working / blocked / done / error` to Zellij through OSC 7501, which this rail
displays as-is. Current-session agents stay listed while idle using Zellij's
pane inventory, even when the attention snapshot omits them. Other emitters
(Codex, Crush, Copilot CLI, Qwen Code, goose, …) work the same way. Programs
that don't emit anything still surface through Zellij's bell/notification/
prompt-marker detection.

## Status / roadmap

- [x] Event-driven rendering, cross-session jump, click navigation
- [ ] Keyboard navigation mode (j/k + Enter) via a `MessagePlugin` keybind
- [ ] Live pane preview (`SetLiveView`)
- [ ] Working spinner (`Timer`)
- [ ] Exited-session list (`SessionUpdate` resurrectable sessions)

## License

MIT
