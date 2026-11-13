# Float controls

## Goal

Give floating plugin panes configurable title-bar buttons and pointer shapes that indicate float interactions. Configure the tabshell close button to hide its float without stopping its tmux shell.

## Constraints

- Herdr owns the float border, hit testing, and pointer changes. Plugins declare their controls; they do not draw over the border.
- A plugin can define multiple buttons with icons and actions. Do not hard-code tabshell behavior into Herdr.
- Keep existing float movement, resizing, focus, and mouse input working when no buttons are configured.
- Preserve the published client endpoint compatibility contract. Do not change the encoding of `endpoint.floats.v1` in place.
- Pointer shapes are terminal-dependent. Unsupported terminals must remain usable.
- Existing plugins without button declarations continue to work.
- The tabshell close action is hide-only and repeat-safe. It targets the clicked float and never shows or creates one. Hiding preserves the float, its geometry, its terminal, and its tmux shell.
- Projecting pointer shapes requested by applications inside panes is outside this feature. Restore the host pointer state when Herdr stops overriding it; do not claim that this restores a pane application's current requested shape.

## Interfaces

- Plugin pane manifest: declare button icons and actions for a floating pane.
- Float client and server: deliver button metadata and dispatch the selected action using a compatible, capability-aware interface.
- Terminal: use the kitty OSC 22 pointer-shape protocol where supported.
- Tabshell manifest or plugin action: define a close button whose action hides the current float.

The exact manifest syntax, action invocation context, protocol negotiation, and fallback for older clients are implementation design decisions. Document them before exposing the interface.

## Invariants

1. Each visible button invokes its configured action for the clicked float and its owning plugin. A button click does not start a move or resize gesture or reach the pane beneath it. Stale requests fail without invoking an action for another float or plugin. Preserve the association across live handoff, and define the behavior when a plugin is reloaded, disabled, or unlinked.
2. A plugin can configure more than one button. Invalid icons, duplicate identifiers, and unknown actions are rejected with actionable errors.
3. With no configured buttons, floats retain their current appearance and interaction behavior.
4. The tabshell close button hides the clicked workspace float. Repeated or delayed activation never reopens or creates a float. Reopening retains the float ID, geometry, terminal, and shell process, and selects the current tab's shell after a tab switch while hidden.
5. The pointer reflects the existing gesture rules: the top row and left border move, the right and bottom borders resize, and the top-right corner moves. The bottom-right corner resizes both dimensions. A button has a button affordance. An active drag keeps its gesture shape until mouse-up, even if the pointer leaves the border.
6. Recompute or restore the pointer when visibility, geometry, stacking, workspace, endpoint, overlay, or mouse-capture state changes, even when the pointer is stationary. Mouse-up ends the gesture override even if the geometry commit is pending. Detach and shutdown restore the prior host state. Unsupported OSC 22 sequences do not prevent ordinary input or rendering, and capability-query replies do not leak into pane input.
7. Button hit testing follows visible stacking order and matches rendered button positions, including narrow floats and truncated titles.
8. Existing `endpoint.floats.v1` clients and servers continue to interoperate. Require positive support for both button metadata and target-aware action invocation. If either is absent, disable buttons without breaking the float or invoking an untargeted action. Clear any separate metadata on endpoint replacement or server restart.
9. A failed or unavailable button action does not close the client or trigger a different button action.

## Tasks

| ID | Status | Task | Invariants |
| --- | --- | --- | --- |
| T1 | x | Design and document button declarations, ownership and handoff, invocation context, activation and cancellation, icon cell-width validation, overflow, failure feedback, and compatible metadata and invocation negotiation. | 1, 2, 3, 7, 8, 9 |
| T2 | x | Implement plugin button declarations and compatible float metadata and action dispatch. | 1, 2, 3, 8, 9 |
| T3 | x | Render and hit-test buttons, including overlaps, narrow floats, and stacked floats. | 1, 3, 7, 9 |
| T4 | x | Implement and restore terminal pointer shapes for hover and active gestures. | 3, 5, 6 |
| T5 | x | Configure the tabshell close button to hide its float. | 1, 4 |
| T6 | x | Verify manifest validation, protocol compatibility, action routing, pointer transitions, and the live tabshell interaction. Cover old and new client-server combinations, partial capability support, two plugins with identical action IDs, multiple workspaces, live handoff, and unavailable plugins. Exercise button press cancellation, disappearing floats, non-left buttons, narrow titles, and wide, combining, and control characters. | 1-9 |
| T7 | x | Close the T6 review gaps before merging noomz/herdr#4. On `feat/float-title-buttons`, run the remaining `just test` scripts (maintenance, ui-hot-path-architecture, integration-assets, docs-contract) and `just bench-render-scale`. Run an old-client/new-server pair and a new-client/old-server pair. Exercise a late asynchronous button failure, a timeout, and a disconnect end to end through the client. Verify that typing reaches the float after reopen and after live handoff. The harness is in `.local/float-controls/`. | 3, 6, 8, 9 |

## Open design questions

- How does Herdr retain the owning plugin and control association across live handoff? Do live floats retain their original declarations or adopt updated ones after plugin reload?
- Which target-aware action interface identifies the clicked float and workspace without letting older servers silently ignore a required target?
- Which capabilities and transport carry button metadata and targeted invocation without changing the frozen float codec? How are unavailable plugins and failed asynchronous actions reported?
- When does a button activate, how is a press cancelled, and does pressing it focus or raise its float? Define behavior for release outside, float removal, and non-left buttons.
- What icon text and terminal-cell widths are allowed? How are buttons laid out when the title bar is too narrow?
- Which terminal environments pass OSC 22 through, and how does Herdr restore the previous host pointer without claiming to project pane application pointer state?
