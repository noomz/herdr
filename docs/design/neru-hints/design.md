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
