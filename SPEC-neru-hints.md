# neru hints

## Goal

Let a user click any herdr control from the keyboard with neru. A herdr client action sends neru the screen positions of herdr's clickable controls, so neru labels those controls and nothing else. When the user picks a label, neru clicks it, and herdr handles the click like any other mouse click. neru also labels each visible pane's text input, at the pane's terminal cursor. neru's own hint hotkey can start the same action: in a host terminal it runs `herdr neru-hints`, which starts the action in the herdr client whose window is focused, and falls back to neru's own hints when no herdr client takes the request.

Background, measurements and rejected approaches are in `docs/design/neru-hints/research.md`. Its PoC section shows why this feature exists. neru's default scan sees only one text area per terminal window. neru's `segment` strategy labels herdr's controls, but in a real session it also labels every word of pane output: 729 targets on one kitty window.

## Constraints

- This is a client patch on the `local` fork branch, not a plugin. Plugin commands run on the server, which never sees the client hit map (`src/app/api/plugins/runtime.rs`), and calibration needs the client's mouse reports. Split the work into a general client API plus a neru plugin only if the change goes upstream.
- neru does the labelling, label input and clicking. herdr does not draw labels or handle hint keys in this feature. herdr's own hint mode is a separate feature (approach C in the research note).
- herdr talks to neru only through the documented `neru hints --targets` interface (neru v1.58.0 or later). It does not depend on neru internals, its IPC socket format, or its config file.
- The selected target's click goes through the host terminal into herdr's existing mouse handling (`ClientShell::handle_mouse`). Do not add a second path that runs control actions.
- The client that receives the keybinding sends the targets, from its own hit map. Other clients attached to the same server are unaffected.
- The feature works only when the client runs on the same Mac as the GUI host terminal. For an SSH client, a client inside another multiplexer, or a missing neru, it reports why it is unavailable and changes nothing.
- herdr does not know where its grid sits on screen. Finding that out (calibration) must not need per-terminal config values for the common case: one kitty or Ghostty window, with or without the host's tab bar and padding, on a display of scale 1 or 2.
- Without the action, herdr behaves exactly as before, including mouse reporting modes and pointer position.
- `neru action move_mouse` does not cross displays: a point on another display lands on the nearest edge of the pointer's current display (research note, finding 12). A host window on a display neru cannot warp to cannot be calibrated, and a pointer that started there cannot be restored.

## Interfaces

- Client action: a new action, bound in the prefix table like other client actions, that sends the current hit map to neru.
- External command: herdr starts neru as a child process and writes the targets JSON to its stdin, `neru hints --targets - --action left_click`. The command returns as soon as hint mode is active (about 20 ms, neru v1.62.0), not when the user picks a label. The neru binary path is configurable; the default is `neru` on `PATH`.
- Targets JSON: neru's documented shape, `{"hints":[{"x","y","width","height","title"}]}`, in global top-left screen points.
- Calibration: move and observe with SGR pixel reports, chosen in T1 (research note, "Calibration measurements", Decision). herdr turns on any-motion SGR pixel reporting, warps the pointer with `neru action move_mouse` to a point inside `neru query window` (window center first, then a 3 × 3 grid until one point reports), then to a second point 40 pt away. The two pixel reports give the scale and the grid origin. herdr then moves the pointer back and restores its mouse modes. It needs no config values.
- Other neru commands used: `neru query window --json`, `neru query cursor --json`, `neru query displays --json`, `neru action move_mouse --x --y`. herdr reads only display rectangles from `query displays`, never its scale (§V.9). They tell a moved pointer from a window on a display neru cannot reach (§V.10), and they drop off-screen targets (§V.11).

- Pane cursors: the server sends each pane's cursor (cell relative to `inner_rect`, and whether it is visible) as an `EndpointControl` message of kind `endpoint.pane-cursors.v1`, data `{boot_id, cursors:[{pane_id, x, y, visible}]}`. It is gated by the endpoint capability `pane_cursors_v1` and a `pane_cursors` flag in the client hello (serde default `false`). The server sends it only when the cursor set changes, in the same render write as a surface or patch. `PaneSurfacePane` does not change (§V.19). A server without the capability (an older herdr on another machine) sends no cursors, and its panes get no targets.

- Config keys: `keys.neru_hints` (binding, default `prefix+f`) runs the action. `neru.path` (default `"neru"`) names the neru binary.

- CLI: `herdr neru-hints` sends API method `client.neru_hints.run` to the local server. The server forwards the request to one client (§V.20) as a server-to-client control, gated by a client hello flag `neru_hints` (serde default `false`). The wire shape, the exit codes and the messages are decided in T13. Exit 0 means a client took the request. Non-zero means none did, with the reason on stderr.
- neru config (user side, documented in T16, not herdr code). A root `[[app_configs]]` entry for each host terminal (`space.noomz.kitty`, `com.mitchellh.ghostty`) binds `"Primary+Shift+Space"` to an `exec` step: `herdr neru-hints || neru hints --action left_click`, with absolute paths. `[hints.hotkeys]` binds `"Primary+Shift+Space" = "idle"`, so a second press closes the labels. This binding is required: in hint mode, neru sends an unbound Cmd chord back to `[hotkeys]` and `[[app_configs]]`, so without it a second press would run `herdr neru-hints` again while the labels are up.

The target list, titles, size budget, config keys and unavailable-case messages are in `docs/design/neru-hints/design.md` (T2).

## Invariants

1. Every target sent to neru is a control in the sending client's current hit map. Pane text, pane bodies and host terminal chrome are never targets, except one pane-cursor target per pane (§V.18).
2. Choosing a label triggers the same action as a real left click on that control. The click's center lands inside the control's rendered cells on every supported host and scale.
3. Each target's title is the control's visible text where it has some (workspace name, tab title, menu row, button label), so neru's search mode can find it.
4. When calibration fails or is not reliable, herdr does not start neru and reports why. It never sends targets it knows to be offset.
5. If the hit map changes between sending and clicking (redraw, resize, tab switch), the click can land on a different control. herdr accepts this risk. It builds the targets on the main thread from the current hit map after calibration returns, immediately before it starts neru. If the grid size or cell pitch changed since calibration started, it does not send them and reports why.
6. The action never blocks the client's render or input loop. A slow, failing or missing neru process is reported and its child process is reaped.
7. Unavailable cases (SSH client, no GUI host, unsupported host, missing or too-old neru) report a clear reason and leave the session untouched.
8. Mouse reporting modes, pixel mouse state and the host pointer return to their previous state after calibration, whether calibration succeeds or fails. If the pointer started on a display neru cannot warp to, herdr reports that it could not restore the pointer.
9. The cell pitch is the ioctl pixel extent divided by the grid size, rounded down, as `ioctl_cell_size` computes it. The scale comes from the two calibration reports, never from `neru query displays`, which reports scale 1 on HiDPI displays.
10. Calibration fails, and reports why, when no candidate point produces a pixel motion report, or when `neru query cursor` does not equal the target point after a warp. This covers a person moving the pointer during calibration and a host window on a display neru cannot reach.
11. A target whose center is outside every display is not sent.
12. Restoring mouse modes re-sends the full mode set the client needs (any-motion, SGR, and pixel mode only if a pane requests it). It never sends `?1016l` alone, because in kitty that also resets the report encoding to legacy X10.
13. A target that contains child targets (a workspace row with its group toggle, a machine row with its collapse toggle or retryable status badge) is sent as the widest run of its cells that no child covers, and is dropped when no cell is left. Its center never lands on a child.
14. While calibration runs, the client swallows every host mouse report, from the moment it turns on pixel reporting until the report from the warp back has arrived or timed out. This includes reports queued before the first warp. None reaches `ClientShell::handle_mouse` or a pane, so calibration warps change no hover state and send no mouse events to pane apps. Only the calibrator reads them.
15. While a `ContextMenu` or `GlobalMenu` overlay is open, the `keys.neru_hints` binding, prefix included, runs the action. It is checked before `route_overlay_key`, and it leaves the menu open, so the menu's rows become the targets. After the prefix, any other key goes to the menu as before. Every other overlay (rename, new tab, worktree dialogs, onboarding, settings, help) keeps taking every key, so the binding does nothing there.
16. Every press of `keys.neru_hints` that does not start neru shows its reason, even when an earlier press failed for the same reason. A neru notice is never suppressed by the once-per-server-boot filter for endpoint notices (`endpoint_notice_seen`). If the same notice is still on screen, it stays visible for its full duration again.
17. A press that ends with the pointer not restored says so in the notice that ends the press, together with any other reason. A later notice never replaces the restore note.
18. Each visible pane whose cursor is visible and inside its viewport (not scrolled back) gets one target: its cursor cell. The title is the pane's border title, else its agent name, else `pane`. There is no target when the cursor is hidden, the pane is scrolled back, a float or the toast covers the cell, an overlay is open, or the pane's server lacks `pane_cursors_v1`. Picking the label is a real click at that cell: it focuses the pane and reaches its app like any click (§V.2).
19. Pane cursors travel only in `endpoint.pane-cursors.v1`. The wire layout of `PaneSurfacePane`, `PaneSurface` and `PaneSurfacePatch` never changes, so generation-1 peers of any version still decode each other. The server sends cursors only to a client whose hello sets `pane_cursors`. The cursor control is appended to the same render write as its surface or patch (full, identical-frame and retained-patch paths), never sent on `writer.control`, so it cannot overtake the surface it describes. The server records a cursor set as sent only after that write succeeds. The client drops a set whose `boot_id` is not its server's.
20. `herdr neru-hints` starts the action in exactly one client: the active shell client of the local server whose host window is focused (`outer_terminal_focus == Some(true)`, `src/server/clients.rs`) and whose hello sets `neru_hints`. When no client qualifies, or more than one does (focus reports can be stale), it changes nothing, prints why, and exits non-zero. It never falls back to the latest or any other client.
21. A request from the CLI runs the same client path as a press of `keys.neru_hints`: availability checks, overlay rule, calibration, targets and notices (§V.4–8, 14–17). Only the trigger differs. herdr does not add a second implementation of the action.
22. The CLI never waits for calibration, neru or a label pick (§V.6). It exits once the server has handed the request to the chosen client, or has refused it. A failure after the hand-off (calibration, neru, an open dialog) is reported as the client's notice, not as the CLI's exit code.
23. A client that does not set `neru_hints` (an older herdr) never receives the request and never counts as focused for §V.20. The server sends nothing new to it.

## Tasks

| ID | Status | Task | Invariants |
| --- | --- | --- | --- |
| T1 | x | Prototype calibration outside herdr. A script maps one known cell to screen points with each candidate method, then clicks it with `neru action move_mouse` and `left_click`. Measure the error in kitty and Ghostty, at scale 1 and 2, with and without the host tab bar and kitty splits. Record the results in the research note and pick a method. | 2, 4 |
| T2 | x | Design and document the target list: which hit-map entries become targets, their titles, how overlays and floats stack, and the size limit. Write the config keys and the unavailable-case messages. | 1, 3, 7 |
| T3 | x | Implement the pure conversion from hit map and calibration to targets JSON. Unit-test it with fixed hit maps, cell sizes, origins and scales. | 1, 2, 3, 9, 11, 13 |
| T4 | x | Implement calibration with the method chosen in T1, including restoring mouse and pointer state on success and failure. | 2, 4, 8, 9, 10, 12 |
| T5 | x | Implement the client action: keybinding, availability checks, starting neru without blocking, writing stdin, reaping the child, and reporting errors. | 5, 6, 7, 14 |
| T6 | x | Verify end to end on a real Mac in kitty and Ghostty, at scale 1 and 2. Check that each label type clicks its control: workspace row, tab, `+`, sidebar toggle, menu row, float button. Also check a busy session (target count matches the hit map), a missing neru, an SSH client, a redraw between sending and clicking, a kitty split, a host window on the second display, and pointer movement during calibration. | 1-14 |
| T7 | x | Let `keys.neru_hints` through `ContextMenu` and `GlobalMenu` overlays before `route_overlay_key` (`src/client/shell/input.rs:653`). Test that the binding runs the action with each menu open and leaves it open, that a non-neru key after the prefix reaches the menu, and that a text-input overlay still takes the keys as text. Rerun the T6 menu-row cell. | 1, 15 |
| T8 | x | Show every neru failure notice on every press (§V.16): `show_neru_notice` must not go through the once-per-boot `endpoint_notice_seen` filter. Test that two presses failing for the same reason (missing neru, calibration failure) each show the notice. Rerun the T6 second-display cell. | 4, 7, 16 |
| T9 | x | Measure where the terminal cursor sits, and whether it is visible, in Claude Code, Codex, a shell prompt, vim and less inside herdr. Record the results in the research note and decide what happens when the cursor is hidden. | 18 |
| T10 | x | Server and protocol: send each pane's cursor behind `pane_cursors_v1`. Test a new client with an old server and an old client with a new server. | 18, 19 |
| T11 | x | Targets: pane-cursor targets in `neru_targets.rs`, with occlusion and titles. Unit-test them with fixed hit maps. | 1, 2, 3, 18 |
| T12 | x | Verify live in kitty and Ghostty: label a pane that is not focused (Claude Code), pick it, and type into its input. | 1, 2, 18 |
| T13 | x | Design `herdr neru-hints` in the design note: server-to-client wire (control kind, capability, hello flag), API method and CLI, client choice, exit codes and messages, a focused client that is showing a remote machine's workspace, and what the client shows when an open dialog blocks the request. | 20, 21, 22, 23 |
| T14 | . | Server, protocol, API and CLI: hello flag, client choice, forwarding, exit codes. Test zero, one and two focused clients, an unfocused client, an old client without the flag, and the CLI exit codes. | 20, 22, 23 |
| T15 | . | Client: run the action on a request through the `keys.neru_hints` path. Test that a request and a key press reach the same entry point, and that a request with an open dialog behaves like the key. | 21, 22 |
| T16 | . | Document the neru config (the `[[app_configs]]` exec binding with fallback, and the required `[hints.hotkeys]` idle binding) in the design note and the user docs. | 20, 22 |
| T17 | . | Verify live in kitty and Ghostty. `cmd+shift+space` in a herdr window shows herdr's labels, and a second press closes them. In another app, and in a terminal window without herdr, neru's own hints run. With two clients, only the focused one acts. | 20, 21, 22 |

## Bugs

| ID | Date | Cause | Fix |
| --- | --- | --- | --- |
| B1 | 2026-11-14 | T1 probe turned pixel mode off with `?1016l` alone. kitty then sent legacy `ESC [ M` reports, which the SGR parser ignored, so calibration saw no motion. | V12 |
| B2 | 2026-11-14 | T1 probe divided Ghostty's ioctl extent (2446 px, 152 columns) as floats. The extent includes leftover padding, so the pitch drifted and a far-edge click landed one column off. | V9 |
| B3 | 2026-11-14 | T6: an open overlay sends every key to `route_overlay_key`, so `prefix+f` never reached the prefix table. Menu rows, the only targets while a menu is open (design note section 2), could not be reached from the keyboard. | V15 |
| B4 | 2026-11-14 | T6: `show_neru_notice` used `push_endpoint_notice` with kind `Unavailable`, which shows each code once per server boot (`endpoint_notice_seen`, `src/client/shell/actions.rs`). The code is `neru_hints:<message>`, so a second press failing for the same reason showed nothing. | V16 |
| B5 | 2026-11-14 | T6: `finish_neru_calibration` showed `POINTER_NOT_RESTORED`, then the calibration error replaced it in the single notice slot (`visible_endpoint_notice`). On a real second display the unrestored pointer was never reported. | V17 |
| B6 | 2026-11-15 | T10 plan: §I put pane cursors in `PaneSurfacePane`. That struct is bincode-encoded by position in `PaneSurface`, `PaneSurfacePatch` and the delta, reuse and scroll codecs, and the generation-1 wire is frozen (`src/protocol/wire.rs:1382`: compatible additions go through `EndpointControl`). A new field breaks old and new peers in both directions. | V19 |

## Open design questions

Resolved by T1 (evidence in the research note, "Calibration measurements") and T2 (`docs/design/neru-hints/design.md`):

- **Calibration method.** Move and observe with SGR pixel reports (see Interfaces). Window bounds plus fixed offsets failed: the offset changed with the tab-bar position, Ghostty titlebar style and padding, and kitty splits. kitty remote control reports no screen positions.
- **Scale.** SGR pixel reports and the ioctl extent are both in device pixels. Measure the scale from the two calibration points (V9).
- **Pixel mouse.** Calibration turns pixel mode on briefly, then restores the pane-requested state by re-sending the full mode set (V12).
- **Which controls.** Only controls a left click at the target's center reaches in the current state, in `handle_mouse` layer order. Panes, splits, scrollbars, dividers and background areas are never targets (design note, sections 1–3).
- **Overlays.** An open overlay is exclusive: only its rows and buttons are targets. An open popup pane gives no targets, and herdr reports that (design note, section 2).
- **Calibration lifetime.** Calibrate on every use. It costs tens of milliseconds (one warp is about 7 ms), and herdr cannot see window moves.
- **Stale targets.** Accept the risk and document it (§V.5). `neru hints --targets -` returns once hint mode is active, so herdr cannot tell when neru closes without `neru watch`, which §I does not list. Holding the layout until `neru watch --event mode_exit` reports the mode closed is a possible later extension, not part of T5.
