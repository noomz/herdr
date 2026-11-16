# neru hints: target list, config and messages (T2)

Status: design for `SPEC-neru-hints.md` §T.2. Nothing here is implemented yet. T3 (conversion), T4 (calibration) and T5 (action) build against it. Source references are to branch `local` at `5a4ea8eb`.

## 1. Rule: a target is a control a left click can reach

neru clicks the center of every rectangle herdr sends. The click goes through the host terminal into the input path a real click takes (§V.2). So herdr sends a hit-map rectangle only if a left click at its center does that control's action **now**, in the current state. A rectangle that exists in the hit map but would do nothing, dismiss an overlay, or reach a pane is not a target.

The conversion (T3) works in this order:

1. Find the active layer from client state, in the order in section 2. Most layers are exclusive: when one applies, the layers below it give no targets.
2. Collect that layer's candidate rectangles with their state guards (section 3).
3. Drop a candidate whose center lies inside a rectangle that takes the click first (a later float, the toast). Section 4 has the occlusion rules.
4. Turn each parent rectangle that contains child targets into its free span (section 4.2).
5. Attach titles (section 5), convert to screen points, drop targets off every display (§V.11), and apply the size budget (section 6).

Zero targets after these steps is a reported case (section 8). herdr does not start neru then.

## 2. Layers, in dispatch order

This list follows the code a left press runs through: `ClientShell::handle_input` (`src/client/shell/input.rs:237`), then `handle_mouse` (`src/client/shell/mouse.rs:683`). T3 tests must pin each step.

| Order | Layer | When it applies | Exclusive | Where |
| --- | --- | --- | --- | --- |
| 1 | Machine status badge | No overlay, no popup, point not on the toast | No (shares the base layer) | `machine_diagnostics.rs:127`, called from `input.rs:237` |
| 2 | Onboarding, product announcement, release notes | `overlay` is one of these | Yes | `mouse.rs:705-846` |
| 3 | Popup pane | `popup_pending`, `popup_terminal_id` or `hits.popup` is set | Yes, and gives **no** targets | `mouse.rs:930-966` |
| 4 | Notification toast | See guard in section 3 | No: it sits on top of every layer below | `mouse.rs:968`, `1025-1043` |
| 5 | Overlay (menus, dialogs, pickers) | Any other `overlay` is set | Yes | `mouse.rs:1411-1805` |
| 6 | Floats | No overlay and no popup (`floats_own_point`, `floats.rs:749`) | No: a float covers the chrome under it | `floats.rs:614` |
| 7 | Mobile layout | `mobile_layout_active()` and no overlay | Yes | `mobile.rs:944` |
| 8 | Base chrome (sidebar, tab bar) | Otherwise | — | `mouse.rs:2010-2215`, `endpoint_navigation.rs:45` |

Layer 3 gives no targets because a popup pane takes every click. herdr reports this case (section 8) and does not start neru.

## 3. Candidates per layer

Every row lists the `ShellHitMap` field (`src/client/shell/state.rs:107`), the guard that must hold, and the title.

### Layer 2: full-screen notices

| Overlay | Field | Title |
| --- | --- | --- |
| Onboarding | `overlay_primary` | drawn label |
| Product announcement | `overlay_primary` | drawn label |
| Release notes | close rect from `current_release_notes_input_geometry()`, else `overlay_primary` | drawn label |

### Layer 4: notification toast

| Field | Guard | Title |
| --- | --- | --- |
| `notification_toast` | `visible_endpoint_notice` is set, **or** no overlay, mode is Terminal, and `visible_notification` has a `pane_id` | drawn text |

### Layer 5: overlays

Clicking outside an overlay closes it. That is not a target: Escape does the same.

| Overlay | Fields | Guard |
| --- | --- | --- |
| `GlobalMenu` | `global_menu_rows`, `global_launcher` | |
| `ContextMenu` | `context_menu_rows` | |
| `PaneTargetSelector` | `pane_target_rows` | row index is in `eligible_pane_ids` |
| `WorktreeCreate` | `overlay_primary`, `overlay_cancel` | `overlay_cancel` only when not busy |
| `WorktreeOpen` | `worktree_search`, `worktree_rows`, `overlay_primary`, `overlay_cancel` | as above |
| `WorktreeRemove` | `overlay_primary`, `overlay_cancel` | as above |
| `Settings` | `settings_tabs`, `settings_choices`, `overlay_primary`, `overlay_cancel` | `overlay_cancel` only when not installing integrations |
| `Help` | `overlay_cancel` | |
| `Navigator` | `navigator_search`, `navigator_rows` | |
| `Rename` | `overlay_primary`, `overlay_clear` | |
| `ConfirmClose` | `overlay_primary` | |

Titles are the drawn row or button text.

### Layer 6: floats

| Field | Guard | Title |
| --- | --- | --- |
| `floats[i].buttons` | layer 6 applies | button `id` from the plugin manifest |

A float's title bar and body are not targets: a press there focuses or drags the float, or reaches its pane.

### Layer 7: mobile layout

| Mode | Fields | Title |
| --- | --- | --- |
| Terminal or Resize | `mobile_switch` | `switch workspace` |
| Navigate (switcher open) | `mobile_targets`, `mobile_close` | drawn row text; `close` |

The mobile layout turns on at or below `ui.mobile_width_threshold` columns (default 64), so a narrow kitty split on a Mac can use it.

### Layer 8: base chrome

| Field | Guard | Title |
| --- | --- | --- |
| `machines[].rect` | | drawn machine name |
| `machines[].collapse_toggle` | width > 0 | `collapse <machine>` or `expand <machine>` |
| `machines[].status_badge` (layer 1) | `machine_retryable` for that endpoint | `retry <machine>` |
| `workspaces[].rect` | | drawn workspace name |
| `workspaces[].group_toggle` | present | `toggle group <group>` |
| `agents`, `endpoint_agents` | | drawn row text |
| `tabs` | | drawn tab title |
| `new_tab` | non-empty | `new tab` |
| `tab_scroll_left`, `tab_scroll_right` | non-empty | `scroll tabs left`, `scroll tabs right` |
| `new_workspace` | non-empty | drawn label |
| `global_launcher` | non-empty | drawn label |
| `sidebar_toggle` | non-empty | `toggle sidebar` |
| `agent_sort_toggle` | non-empty | drawn label |

### Never targets

These fields are clickable but are not controls, or a single click at their center does not do one defined thing (§V.1):

- `panes`, `popup`, `pane_splits`, and float bodies: pane input and split drags. The one exception is a tiled pane's visible cursor cell (§V.18). It is a target after every chrome control and below floats, the toast, split handles and scrollable scrollbars. Its title is the drawn border title, then the agent name, then `pane`.
- all scrollbars and their metrics: `workspace_scrollbar`, `agent_scrollbar`, `help_scrollbar`, `navigator_scrollbar`, `product_announcement_scrollbar`, `release_notes_scrollbar`
- drag handles: `sidebar_divider`, `sidebar_section_divider`
- background areas: `workspace_body`, `agent_body`, `tab_bar`, `help_popup`, `settings_popup`, `navigator_popup`
- links inside pane text

The `*_scroll_metrics` and `*_max_scroll` fields hold scroll state, not rectangles, so they are never targets.

## 4. Stacking and occlusion

### 4.1 Layers that cover others

A candidate is dropped when its center lies inside a rectangle that takes the click first:

- **Toast.** When the toast is a target (or is drawn at all), it covers every float button and base-chrome control under it. Floats check this too (`floats_own_point`).
- **Floats.** `hits.floats` is bottom to top. A button of float `i` is dropped if its center is inside `floats[j].outer` for any `j > i`. A base-chrome candidate is dropped if its center is inside any float's `outer`.
- **Overlays** are exclusive (layer 5), so nothing below them is collected.

### 4.2 Parent and child rectangles

Two parents contain child targets:

- `WorkspaceHit.rect` contains `group_toggle`, the rightmost cell of the row (`sidebar.rs:612`).
- `MachineHit.rect` contains `collapse_toggle` (leftmost cell) and, when retryable, `status_badge` (right edge, `endpoint_sidebar.rs:93-113`).

The center of a parent can land on a child, so a click on the parent label would do the child's action. Rule: **send a parent as the widest run of its cells that no child target covers. If no cell is left, drop the parent.** Children are one row and sit at the left or right edge, so the free span is one rectangle, and its center is a parent-only cell. A badge that is not retryable is not a target, so its cells stay in the parent's span: a click there falls through to the row (`machine_diagnostics.rs:155`).

This rule needs a new spec invariant. It is proposed in section 9.

## 5. Titles

§V.3 asks for the control's visible text, so neru's search mode can find it.

- **Source.** The hit map stores IDs (`tab_id`, `workspace_id`, a menu index), not the drawn text, and the client keeps no composed frame after `compose`. T3 records each control's drawn label in the hit map when render draws it, the way `FloatHit.buttons` already ties hit rects to the drawn layout (`floats.rs:84-91`). The label then cannot drift from the screen. A field that has no recorded label uses the fixed title in section 3.
- **Icon-only controls** (`+`, `«`, `‹`, `›`, `▸`, float button icons) get the fixed descriptive titles in section 3. A glyph is visible, but nobody can type it into a search.
- **Cleanup.** Trim the title, collapse runs of whitespace, remove control characters, and cut it to 64 characters. Truncated drawn text (with `…`) is sent as drawn.
- An empty title is allowed by neru but not sent by herdr: every row in section 3 has a source.

## 6. Size limit

neru's daemon refuses an IPC command over `MaxCommandBytes = 64 << 10` (`internal/adapter/ipc/ipc.go:50` at neru `e108948a`). `neru hints --targets -` forwards the targets JSON as one string argument inside that command. String encoding escapes each `"` and `\`, and Go's JSON encoder writes `<`, `>` and `&` as six-byte `<` escapes.

- **Budget.** herdr measures the targets JSON as an escaped string (2 bytes per `"` and `\`, 6 bytes per `<`, `>`, `&`, other bytes as is) and keeps it at or below **56 KiB**. The other 8 KiB covers the verb, other arguments and the envelope.
- **Typical size.** A target is about 90 bytes plus its title, so the budget holds about 400 targets with 64-character titles. A real session has about 30 to 100.
- **Overflow.** Targets keep the order in section 3. When the budget runs out, herdr drops targets from the end of that order, sends the rest, and reports `sent N of M controls` (section 8). Every target it sends still meets §V.1 and §V.2.

## 7. Config keys

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `keys.neru_hints` | binding | `prefix+f` | Runs the action. `prefix+f` is unbound in the default key table. Set it to `[]` to unbind. |
| `neru.path` | string | `"neru"` | The neru binary, resolved on `PATH` when it has no `/`. |

The minimum neru version (1.58.0), the calibration timeouts and the 40 pt second point are fixed values, not config. Calibration needs no config (§I).

`keys.neru_hints` reaches the client shell through the existing keybind config (`src/config/keybinds.rs`, `src/config/model.rs`), like other client actions. T5 adds the `[neru]` table to `Config` (`src/config/model.rs:320`).

## 8. Messages

herdr reports each case once, as a client notice, and changes nothing else (§V.7). It checks them in this order, cheapest first, and stops at the first that applies. Steps 1 to 6 run before herdr starts any process.

| # | Case | Detection | Message |
| --- | --- | --- | --- |
| 1 | Not macOS | build target | `neru hints needs macOS.` |
| 2 | SSH client | `SSH_CONNECTION` or `SSH_TTY` in the client's environment | `neru hints is unavailable over SSH. Run herdr on the Mac that shows the terminal.` |
| 3 | Inside another multiplexer | `TMUX`, `STY` or `ZELLIJ` set, or the client runs inside a herdr pane | `neru hints is unavailable inside <tmux \| screen \| zellij \| herdr>.` |
| 4 | Mouse capture off | `ui.mouse_capture = false`, or host mouse capture is not active | `neru hints needs mouse capture (ui.mouse_capture).` |
| 5 | No pixel mouse in the host | the `?1016` probe in `terminal_setup.rs` did not report support | `neru hints needs a terminal that reports pixel mouse positions (kitty, Ghostty).` |
| 6 | Popup open, or nothing to click | layer 3, or zero targets after section 1 | `Close the popup first.` / `No clickable controls on screen.` |
| 7 | neru missing | starting `neru.path` fails with not found | `neru not found at "<path>". Install neru 1.58.0 or later, or set neru.path.` |
| 8 | neru too old | `neru --version` below 1.58.0 | `neru <version> is too old. neru hints needs 1.58.0 or later.` |
| 9 | neru not running or no permission | `neru query window --json` exits non-zero | `neru query window failed: <first stderr line>` |
| 10 | No motion report (§V.10) | no candidate point gives a pixel report | `Calibration failed: the terminal sent no mouse report.` |
| 11 | Pointer moved (§V.10) | `neru query cursor` is not the warp target | `Calibration failed: the pointer moved. Keep the mouse still and try again.` |
| 12 | Window on another display (§V.10) | the warp lands on a display edge | `Calibration failed: neru cannot move the pointer to this window's display.` |
| 13 | Bad scale (§V.4) | the two reports give a scale that is not positive | `Calibration failed: inconsistent mouse reports.` |
| 14 | Pointer not restored (§V.8) | the start point is on a display neru cannot reach | `Could not move the pointer back: it started on another display.` (also after success) |
| 15 | Over budget | section 6 | `neru hints: sent <N> of <M> controls.` (neru still starts) |
| 16 | neru failed | `neru hints` exits non-zero | `neru hints failed: <first stderr line>` |

Step 5 uses the capability probe, not a list of terminal names, so any host that reports pixel positions works. Calibration (steps 10 to 13) is the final check.

## 9. Spec changes this design needs

These go through the spec skill, not build:

- **New invariant (parent and child rectangles):** a target that contains child targets is sent as the widest run of its cells that no child covers, and is dropped when none is left, so its center never lands on a child (section 4.2).
- **Resolve "Which controls"** with sections 1 to 3: only controls a left click can reach now; panes, splits, scrollbars, dividers and backgrounds never.
- **Resolve "Overlays"** with section 2: an open overlay is exclusive, and only its rows and buttons are targets. A popup pane gives no targets.
- **Interfaces:** add the config keys from section 7.

"Stale targets" stays open for T5.

## 10. `herdr neru-hints` (T13)

Status: design for §T.13. T14 (server, protocol, API, CLI), T15 (client) and T16 (neru config) build against it. Source references are to branch `local` at `54a0a09c`.

neru's hint hotkey runs `herdr neru-hints || neru hints --action left_click` (§I). The command asks the local server to start the action in the herdr client whose window is focused. It exits 0 once one client has the request. Otherwise it exits non-zero, and the `||` runs neru's own hints. The precedent is `machine.show`: an API method that the server forwards to one client as an `EndpointControl` (`src/server/headless/machine_show.rs`, `src/client/endpoint/control.rs:35`).

### 10.1 Flow

1. neru runs `herdr neru-hints`. The CLI sends `client.neru_hints.run` to the local server.
2. The server picks one client (10.4). It sends that client `endpoint.neru-hints.v1` and answers `Ok {}`. If no client qualifies, it answers an error.
3. The CLI exits 0 on `Ok`, or prints one line and exits 1 (10.5). It does not wait for the client (§V.22).
4. The client runs the action through the key path (10.7). Every later failure is a client notice.

### 10.2 Wire

| Item | Direction | Shape | Gate |
| --- | --- | --- | --- |
| Hello flag `neru_hints` | client → server, in `EndpointClientHello` | `bool`, `#[serde(default)]` | none: an old server ignores the field |
| Capability `neru_hints_v1` | server → client, in welcome `capabilities` | string | none |
| `endpoint.host-focus.v1` | client → server, `EndpointControl` | `{"focused": bool}` | server advertised `neru_hints_v1` |
| `endpoint.neru-hints.v1` | server → client, `EndpointControl` | `{}`; the client ignores unknown fields | client hello set `neru_hints` |

- **Hello flag.** The client sets `neru_hints` only in its hello to `ClientEndpointId::Local` (`handshake.rs` already takes `local_transport`). It also requires section 8 steps 1 to 3 to pass at startup: macOS, no SSH, not nested. A client that cannot run neru then never counts as focused. A stale focus report from an SSH client cannot make the CLI refuse as ambiguous. §V.23 allows this. The client still runs the full chain in section 8 when a request arrives.
- **Why a capability.** `endpoint.neru-hints.v1` needs no capability, because the client opted in with the hello flag. `endpoint.host-focus.v1` goes the other way, so the client sends it only to a server that lists `neru_hints_v1`. An older server never receives a control kind it does not know.
- **Why a separate focus bit.** `ClientShellFocus` drives the server's PTY focus events (`ServerEvent::ClientShellFocus`, `src/server/headless.rs:2380`). It goes only to the active endpoint. When the client switches to a remote machine, it sends `ClientShellFocus { focused: false }` and surface-off to the local server (`src/client/endpoint/activation.rs:150-160`). `outer_terminal_focus` therefore cannot tell whether the host window holding that client is focused. `endpoint.host-focus.v1` reports only the host window. It changes no PTY focus.
- **Sending host focus.** The client sends `endpoint.host-focus.v1` to the local endpoint once after the welcome, and again on each host focus change. Many hosts send no focus report until the window loses focus once. With no report yet, the first value is `focused: true`, the rule herdr already uses: `src/client/mod.rs:649` sends `ClientShellFocus { focused: true }` at startup, and `src/client/shell/endpoints.rs:476` reads `outer_focused.unwrap_or(true)`. A new client can then answer the first press. The cost is the same as today: a client started in the background at the same moment can make the CLI refuse as ambiguous, which is safer than a client that never qualifies. It sends it whatever endpoint is active, including during an endpoint activation. The send sits where `shell_runtime.rs:881` routes `ClientShellFocus`, before the activation and active-endpoint gates.
- **Server state.** `ClientConnection` gains `shell_neru_hints: bool` (from the hello) and `host_window_focused: Option<bool>` (from `endpoint.host-focus.v1`, `None` until the first report). Neither field changes PTY focus, foreground client or tab geometry.
- **Origin check.** The client acts on `endpoint.neru-hints.v1` only when it arrives from `ClientEndpointId::Local`, and drops it from any other endpoint. It never sets the flag in a hello to a remote endpoint either, so `herdr neru-hints` on another machine can never start calibration on this Mac.

### 10.3 API method and CLI

- **Method.** `client.neru_hints.run` takes `EmptyParams` and returns `ResponseResult::Ok {}`. It is added to `src/api/schema.rs` beside `client.window_title.*`, and handled before the generic request path, like `MachineShow` (`src/server/headless.rs:3102`).
- **CLI.** `herdr neru-hints` takes no arguments. It is a top-level command in `src/cli.rs`, beside `title`. It uses `send_request`, so it pings for protocol compatibility first. That is two local socket round trips, a few milliseconds.
- **Server.** The CLI uses the local API socket: `HERDR_SESSION` if set, else the default session (`ApiClient::local()`). A client in a named session is reachable only when the neru `exec` line sets `HERDR_SESSION` (T16).
- **No remote target.** The command refuses `--machine` and any saved-machine target (`target::is_remote()`): `herdr neru-hints runs only against the local server.` The neru hotkey runs on this Mac.
- **No autostart.** When no server is listening, the CLI fails at once with the existing `server_not_running` error (`map_server_not_running_or_io`, `src/cli.rs`). It never starts a server. The hotkey runs in every terminal window, most of them without herdr.

### 10.4 Client choice (§V.20)

The server sends the request to the one client for which all of these hold:

- `is_shell_client()`
- `shell_neru_hints`
- `writer.is_some()`
- `host_window_focused == Some(true)`

It does not require `shell_surface_active`. A client that shows a remote machine still qualifies (10.6).

| Qualifying clients | Result |
| --- | --- |
| 0 | error `no_focused_client` |
| 1 | send `endpoint.neru-hints.v1`. If `send_to_client` fails, error `no_focused_client` |
| 2 or more | error `ambiguous_focused_client`, nothing sent. Focus reports can be stale, so herdr does not guess (§V.20) |

### 10.5 Exit codes and messages

The CLI prints one plain line on stderr, not the JSON that `send_ok_request` prints, because neru's log shows it as is.

| Case | Exit | stderr |
| --- | --- | --- |
| A client took the request | 0 | nothing |
| `no_focused_client` | 1 | `herdr neru-hints: no focused herdr window.` |
| `ambiguous_focused_client` | 1 | `herdr neru-hints: <N> herdr windows report focus. Click one and try again.` |
| No server listening | 1 | `herdr neru-hints: no herdr server is running.` |
| Old server: error code `invalid_request` (it cannot parse the method, `src/api/server.rs:367`), or `unknown_method` | 1 | `herdr neru-hints: the herdr server does not support neru-hints. Restart it after updating.` |
| Protocol mismatch | 1 | the existing message from `ensure_server_protocol_compatible`, which prints it itself (`ProtocolMismatchReported`, `src/cli/protocol_guard.rs`). The CLI adds no line. |
| Remote target | 2 or 1 | `--machine` fails the shared target check before any connection (`validate_machine_command`, `src/cli/target.rs`): exit 2 and its usage line. Any other remote target: exit 1, `herdr neru-hints runs only against the local server.` |
| Any other error | 1 | `herdr neru-hints: <server message>` |

There is one non-zero code. The neru `exec` step tests only success, so different codes would carry no meaning.

### 10.6 A focused client that shows a remote machine

The client is attached to the local server and also shows a saved machine's workspace. It stays connected to the local endpoint, with a writer, while it shows the remote machine. The local endpoint is disconnected only on a catalog reload (`src/client/catalog_reload.rs:47`). With 10.2, its host-focus bit stays true, so the server picks it. The client then runs the action on what it shows. The hit map is the client's own, so the controls are those of the remote view. Pane-cursor targets come from the remote endpoint's `endpoint.pane-cursors.v1` only if that server has `pane_cursors_v1` (§V.18).

Without 10.2, this case would fall back to neru's own hints, the 729-target result the feature exists to avoid (research note, PoC). A key press would still work in the same state, which breaks §V.21.

### 10.7 What the client does on a request (§V.21)

The control decodes to `EndpointControlMessage::NeruHints` and calls one new method, `ClientShellState::request_neru_hints(&mut outcome)`. It applies the overlay rule a key press meets (§V.15):

| Client state | Action |
| --- | --- |
| No overlay, or `ContextMenu` / `GlobalMenu` open | End Prefix mode if the client is in it, as the menu path in `input.rs` does. Then `record_binding(KeybindMatch::Action(KeybindAction::NeruHints))`, the call a key press makes. It sets `neru_hints_requested` (`src/client/shell/actions.rs:177`). A menu stays open, and its rows become the targets. |
| Any other overlay (rename, new tab, worktree dialogs, onboarding, settings, help) | `show_neru_notice("Close the dialog first.")`. Nothing else changes. |

- **Why a notice for a dialog.** A key press in a dialog is typed into it as text, so the user sees what happened. A request has no key, and the CLI has already exited 0, so neru's own hints do not run either. Without a notice, the press would do nothing visible. §V.16 asks for a reason on every press that does not start neru, and §V.22 names an open dialog as a client notice.
- **One entry point.** From `neru_hints_requested` on, the request and the key are the same. `take_neru_hints_request` is polled at the top of each client loop iteration (`src/client/mod.rs:801`), before the loop waits in `select!` (`:844`). A request decoded in one iteration therefore starts in the next one, without waiting for input. All availability checks, calibration, targets and notices (section 8) run unchanged.
- **Other modes.** Copy mode and resize mode are left as they are. The action runs as it does for the key.

### 10.8 Spec changes this design needs

These go through the spec skill, not build:

- **§V.20:** "active shell client … `outer_terminal_focus == Some(true)`" becomes "a shell client of the local server whose hello sets `neru_hints` and whose last `endpoint.host-focus.v1` reported focused". The new wording includes a client that shows a remote machine (10.6).
- **§V.23:** add that the client acts on `endpoint.neru-hints.v1` only from its local endpoint, and sets `neru_hints` only in the local hello and only when section 8 steps 1 to 3 pass.
- **§I CLI bullet:** add capability `neru_hints_v1`, control kinds `endpoint.neru-hints.v1` (server → client) and `endpoint.host-focus.v1` (client → server), and the exit codes in 10.5. State that the CLI never starts a server.
- **§T.14 tests:** add a focused client that shows a remote machine, a new client with no focus report (it qualifies), host focus against a server without `neru_hints_v1`, a request from a remote endpoint that the client drops, and a live handoff (`src/server/handoff.rs`) after which `shell_neru_hints` and `host_window_focused` are kept or rebuilt from a new hello and the post-welcome send.
- **§T.15 tests:** add a request that arrives while a calibration or neru child from an earlier press is still running. A quick double press can do this before neru's `[hints.hotkeys]` idle binding takes effect. The request does what a second key press does in that state.
