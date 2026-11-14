# neru hints

## Goal

Let a user click any herdr control from the keyboard with neru. A herdr client action sends neru the screen positions of herdr's clickable controls, so neru labels those controls and nothing else. When the user picks a label, neru clicks it, and herdr handles the click like any other mouse click.

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

## Interfaces

- Client action: a new action, bound in the prefix table like other client actions, that sends the current hit map to neru.
- External command: herdr starts neru as a child process and writes the targets JSON to its stdin, `neru hints --targets -`. The neru binary path is configurable; the default is `neru` on `PATH`.
- Targets JSON: neru's documented shape, `{"hints":[{"x","y","width","height","title"}]}`, in global top-left screen points.
- Calibration: whatever input herdr needs to map grid cells to screen points. Candidates are listed under Open design questions.

Choose and document the calibration method, the target list, and the config keys before exposing the action.

## Invariants

1. Every target sent to neru is a control in the sending client's current hit map. Pane text, pane bodies and host terminal chrome are never targets.
2. Choosing a label triggers the same action as a real left click on that control. The click's center lands inside the control's rendered cells on every supported host and scale.
3. Each target's title is the control's visible text where it has some (workspace name, tab title, menu row, button label), so neru's search mode can find it.
4. When calibration fails or is not reliable, herdr does not start neru and reports why. It never sends targets it knows to be offset.
5. If the hit map changes between sending and clicking (redraw, resize, tab switch), the click can land on a different control. The action keeps that window small and its behavior defined. See the open question on stale targets.
6. The action never blocks the client's render or input loop. A slow, failing or missing neru process is reported and its child process is reaped.
7. Unavailable cases (SSH client, no GUI host, unsupported host, missing or too-old neru) report a clear reason and leave the session untouched.
8. Mouse reporting modes, pixel mouse state and the host pointer return to their previous state after calibration, whether calibration succeeds or fails.

## Tasks

| ID | Status | Task | Invariants |
| --- | --- | --- | --- |
| T1 | x | Prototype calibration outside herdr. A script maps one known cell to screen points with each candidate method, then clicks it with `neru action move_mouse` and `left_click`. Measure the error in kitty and Ghostty, at scale 1 and 2, with and without the host tab bar and kitty splits. Record the results in the research note and pick a method. | 2, 4 |
| T2 | | Design and document the target list: which hit-map entries become targets, their titles, how overlays and floats stack, and the size limit. Write the config keys and the unavailable-case messages. | 1, 3, 7 |
| T3 | | Implement the pure conversion from hit map and calibration to targets JSON. Unit-test it with fixed hit maps, cell sizes, origins and scales. | 1, 2, 3 |
| T4 | | Implement calibration with the method chosen in T1, including restoring mouse and pointer state on success and failure. | 2, 4, 8 |
| T5 | | Implement the client action: keybinding, availability checks, starting neru without blocking, writing stdin, reaping the child, and reporting errors. | 5, 6, 7 |
| T6 | | Verify end to end on a real Mac in kitty and Ghostty, at scale 1 and 2. Check that each label type clicks its control: workspace row, tab, `+`, sidebar toggle, menu row, float button. Also check a busy session (target count matches the hit map), a missing neru, an SSH client, and a redraw between sending and clicking. | 1-8 |

## Bugs

| ID | Date | Cause | Fix |
| --- | --- | --- | --- |

## Open design questions

- **Calibration method.** Candidates, to settle in T1:
  - Move and observe. herdr runs `neru action move_mouse` to a known screen point inside the window, and reads the cell (or the pixel, with SGR pixel mouse on) from the next motion report. That gives the grid origin directly. It moves the user's pointer, and cell-only reports leave up to half a cell of error unless two points are used.
  - Query neru's window bounds (`neru query window`) and add fixed host offsets for title bar, tab bar and padding. Cheap, but breaks with kitty splits, host tab bars and changed padding.
  - Ask the host terminal for its window geometry (kitty remote control). kitty only, and unverified whether it reports screen origins.
- **Scale.** Is the ioctl cell size in device pixels or points on Retina displays? Take the scale from `neru query displays`, or measure it with two calibration points?
- **Pixel mouse.** SGR pixel mouse is on only while a pane requests it (`effective_sgr_pixel_mouse`, `src/client/terminal_setup.rs:630`). May calibration turn it on briefly, and how is the pane's requested state restored?
- **Stale targets.** neru exits hint mode on a screen change but not on a herdr redraw. Should herdr hold its layout while neru is open (and how would it know neru closed), or accept the risk and document it?
- **Which controls.** Every hit-map entry, or a curated set? Pane bodies and splits are clickable but are not controls. Mobile layout targets probably never apply on a Mac.
- **Overlays.** While a menu or popup is open, should only its rows be targets, matching what a real click can reach?
- **Calibration lifetime.** Calibrate on every use, or cache per client until the host window moves or resizes? herdr cannot see window moves.
