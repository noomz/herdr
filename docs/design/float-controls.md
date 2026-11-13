# Float control contracts

Status: T1 design for `SPEC-float-controls.md`, with T2 implementation notes appended at the end. Contracts without a T2 note still describe proposed behavior, not implemented features.

## Scope and existing boundaries

Herdr owns float borders and input. Plugins declare ordered buttons that invoke their own actions. The server owns bindings, validation, and command execution. The client owns button layout, hover, and press state.

The current boundaries are:

- `src/app/api/plugins/manifest.rs` parses TOML pane declarations and validates action references for link handlers.
- `src/api/schema/plugins.rs` defines pane declarations, action invocation, and plugin context.
- `src/app/api/plugins/panes.rs::open_plugin_float` spawns a float but does not retain a `PluginPaneRecord` for it.
- `src/app/state.rs::PluginPaneRecord` associates ordinary plugin panes with a plugin ID and entrypoint.
- `src/app/floats.rs` exports and restores live floats. `src/handoff_runtime.rs::HandoffFloat` carries geometry, workspace association, and runtime state, but no plugin binding.
- `src/protocol/floats.rs` transports `endpoint.floats.v1` with bincode. Its fields and reachable enums are frozen.
- `src/protocol/endpoint.rs` negotiates optional JSON hello fields, capabilities, and API method names.
- `src/server/headless/endpoint_requests.rs` checks server boot, active surface, and endpoint method eligibility before dispatch.
- `src/app/api/plugins/context.rs::merge_plugin_context` fills unspecified fields from the active workspace. Targeted button actions must not use that fallback to determine their target.
- `src/app/api/plugins/runtime.rs::start_plugin_command` returns a command log and runs the command asynchronously with `HERDR_PLUGIN_CONTEXT_JSON`.
- `src/client/shell/floats.rs` composes floats bottom to top and implements client-local focus and drag previews.
- `src/client/shell/endpoint_notices.rs` renders existing endpoint failure notices.

T1 changes no Rust interfaces, published docs, plugin manifests, or endpoint fixtures. T2–T6 implement and verify the contracts below.

## Pane declarations

Add an optional `buttons` list to each plugin pane declaration. Omission and an empty list mean no controls. The declaration order is the display order, left to right.

```toml
[[panes]]
id = "shell"
title = "Shell"
placement = "float"
command = ["./shell"]
buttons = [
  { id = "inspect", icon = "?", action = "inspect-shell" },
  { id = "close", icon = "×", action = "hide-shell" },
]

[[actions]]
id = "inspect-shell"
title = "Inspect shell"
contexts = ["workspace"]
command = ["./inspect-shell"]

[[actions]]
id = "hide-shell"
title = "Hide shell"
contexts = ["workspace"]
command = ["./hide-shell"]
```

The example is illustrative, not a tabshell patch. Actions remain argv commands, not shell text supplied by the client.

Each button has exactly three required fields: `id`, `icon`, and `action`. IDs use the existing action-ID grammar and length limit. Button IDs are unique within one pane declaration. The action names an action in the same plugin. Two plugins may use the same button or action IDs without sharing ownership.

Validate declarations during manifest normalization, before a plugin becomes available. Reject unknown actions, duplicate button IDs, and invalid icons. Errors identify the plugin, pane, button index or ID, field, and expected value. An unknown-action error names the unresolved action.

Buttons are effective only when the pane opens as a float, including a placement override. Other placements ignore the valid declaration without adding controls. Validate the declaration regardless of its default placement. This preserves existing placement overrides without silently accepting malformed buttons.

Apply existing platform checks to both the pane and its action. An action unavailable on the current platform is not advertised as a usable button. Do not substitute an action with the same ID from another plugin.

### Icon validation

Use the existing `unicode-segmentation` and `unicode-width` dependencies. Accept exactly one extended grapheme cluster with a computed width of one or two terminal cells.

Reject empty text, whitespace-only clusters, leading standalone combining marks, controls, ESC, line breaks, and Unicode format controls. Reject variation selectors and zero-width joiners for this initial contract to avoid terminal-dependent emoji presentation and invisible formatting. A base character followed by ordinary combining marks is valid when the resulting cluster has width one or two. No normalization or trimming changes the supplied icon.

Examples include a one-cell ASCII `x`, a two-cell CJK character, and a base letter with a combining accent. A combining accent alone, a tab, an escape sequence, or two separate letters is invalid.

The client repeats boundary validation on metadata and recomputes width with the same rules. It never writes unvalidated icon text to the host terminal. Font appearance and ambiguous-width terminal settings remain terminal-dependent. Tests assert the cell model used by Herdr, not a specific font's pixels.

## Server-owned bindings and lifecycle

Introduce an optional `FloatPluginBinding` on the server's float state. Do not infer ownership from terminal titles, process environment, argv, or the focused pane.

A binding contains:

- The installed plugin ID and a registry-generated installation identity that changes on unlink and relink, even when the path and plugin ID are reused.
- The pane entrypoint ID.
- An opaque controls revision, regenerated whenever the effective declarations, action definitions, or availability change.
- The normalized button declarations and resolved action definitions needed to compare a restored binding with the current registry.

Use typed float IDs internally. Parse public IDs and opaque revisions at the API boundary. Keep immutable validated declarations separate from client press state. This applies Type System Discipline: a validated binding is distinct from untrusted metadata or invocation parameters.

Create the binding atomically with a plugin float. Non-plugin floats have no binding. Removing a float removes its binding. Hide and show retain the binding and terminal runtime.

On a successful plugin reload, existing floats adopt the current declaration for the same installation and entrypoint. Regenerate their controls revisions and cancel any older presses or requests. Resolve actions from the validated current definition, never from an action ID alone. A rejected reload must not install partially validated declarations.

Disabling a plugin, an unavailable manifest, a removed entrypoint, or an invalid current definition makes its controls unavailable. Keep the terminal and binding identity, but publish no actionable buttons. Re-enabling the same installation creates a new controls revision. Unlink permanently invalidates that installation's binding. Relinking the same plugin ID does not claim its old floats. New floats receive the new installation identity.

Extend the private JSON handoff record with an optional plugin binding, with defaults for older records. Transfer the installation identity and binding together. The importer validates the binding against its plugin registry before advertising controls. Matching installations adopt their current declarations and receive a fresh controls revision. Missing or mismatched installations retain a usable terminal with no buttons. Never guess ownership for a legacy record that lacks the binding.

Preserve the float ID, geometry, visibility, workspace association, and runtime through supported live handoff. Boot changes invalidate client requests even when the public float ID survives. A handoff through an older binary that cannot carry the binding loses controls safely, not terminal usability. Cold restart follows existing float restoration behavior and must not invent ownership.

## Metadata negotiation and transport

Keep `endpoint.floats.v1` byte-for-byte unchanged. Do not append fields to `FloatSurface`, alter frozen reachable enums, or update old digests to bless a change.

Add a default-false `float_buttons` field to the JSON endpoint hello. Add a server capability named `float_buttons_v1`. That capability promises the metadata and completion controls described here. Advertise the distinct JSON API method `float.button.invoke` when implemented and allowed on the endpoint request lane.

A client enables controls only when all of these are true:

1. The float layer itself is negotiated.
2. The client opted in with `float_buttons: true`.
3. The welcome advertises `float_buttons_v1`.
4. The welcome advertises `float.button.invoke`.
5. Valid metadata for the current endpoint and server boot is available.

Neither the capability nor the method alone enables buttons. No fallback calls `plugin.action.invoke`.

Send metadata as JSON in a separate named control, `endpoint.float-buttons.v1`:

```json
{
  "boot_id": "server-boot",
  "revision": 12,
  "floats": [
    {
      "float_id": "f7",
      "workspace_id": "w2",
      "plugin_id": "example.shell",
      "entrypoint": "shell",
      "controls_revision": "opaque-binding-revision",
      "buttons": [
        {"id": "close", "icon": "×", "action": "hide-shell"}
      ]
    }
  ]
}
```

`workspace_id` is required but nullable. Null denotes a float shared across workspaces, not the client's active workspace. The frame geometry and terminal content remain exclusively in the existing float layer.

Each metadata message is a full replacement for that connection's currently presented float set. An empty list clears all controls. Revisions increase within one boot. Ignore older revisions. Reject malformed metadata as an optional-feature error, clear controls, and keep the connection usable. Apply existing frame-size limits and bound all decoded collections by the enclosing message limit.

Only join metadata to float IDs present in the latest float layer for that same endpoint. Send layer updates before their companion metadata over the ordered connection. Removal from either list removes its controls immediately. A delayed layer may briefly display a float without buttons. It must not cause metadata to attach to a different float. On binding invalidation, the server rejects stale requests even before the client sees replacement metadata.

Clear metadata, press capture, pending button tracking, and capability state on endpoint replacement, disconnect, or server boot change. Do not copy controls between endpoints with matching `f<n>` IDs. Metadata is generated on relevant state changes and retained between updates, not by reading manifests inside pane-scaled render loops.

## Target-aware invocation

Add a new API operation `float.button.invoke`, available through the ordinary JSON API and the advertised endpoint request lane. Do not change the parameter shape or meaning of `plugin.action.invoke`, `float.hide`, or another existing method.

Required parameters are:

```json
{
  "boot_id": "server-boot",
  "float_id": "f7",
  "workspace_id": "w2",
  "plugin_id": "example.shell",
  "entrypoint": "shell",
  "controls_revision": "opaque-binding-revision",
  "button_id": "close"
}
```

The method has no client-supplied action ID or argv. All fields are required, including a nullable workspace. The inner boot ID also protects ordinary API callers. Endpoint calls must additionally match the request envelope's boot ID.

Before accepting an action, the server checks the boot, float existence, exact workspace association, plugin and entrypoint binding, installation availability, controls revision, declared button ID, and action platform support. A hidden but still-existing float may accept an already-issued activation if every identity check passes. That permits a delayed hide action to remain hide-only. A removed float or changed binding fails without spawning a command.

Resolve the command from the validated binding and current registry in the same server turn that accepts the request. Capture the target context before starting asynchronous work. Later focus changes must not retarget the command.

Construct a new button invocation context wrapper containing the existing plugin context plus a required `float_target` object with the boot, float, workspace, plugin, entrypoint, controls revision, and button IDs. Serialize that wrapper into `HERDR_PLUGIN_CONTEXT_JSON` for this new operation only. Set `invocation_source` to `float_button` and use the request correlation ID. Do not extend `PluginInvocationContext` in place if doing so changes an existing advertised method's schema digest.

For a workspace float, derive ordinary workspace details and the current tab from that workspace, not the globally active workspace. For a shared float, leave workspace and tab context absent. Never present the underlying tiled pane as the clicked float. The required `float_target` is authoritative. Expose `HERDR_FLOAT_ID` and `HERDR_FLOAT_BOOT_ID` for scripts, alongside the existing plugin environment. Set those values from the validated target, not caller-provided environment.

The invocation response reports the command log ID and its initial status. Acceptance is not command success. Existing plugin command concurrency limits remain in force. Do not retry on timeout or reconnect because an arbitrary action need not be repeat-safe.

### Completion and failure feedback

Immediate validation or dispatch failures use the existing endpoint notice presentation. Define distinct errors for stale boot or controls, target mismatch, unknown button, and unavailable plugin. Include the float and button identity in diagnostic text. Never reinterpret an error as a request for another action.

For negotiated button clients, add `endpoint.float-button-result.v1`, a JSON control correlated with the initiating endpoint request and command log ID. It carries the boot ID, float ID, button ID, final status, exit code, and a bounded error summary. Send completion only to the initiating connection. Route completion from the existing command-finished event rather than blocking the App event loop or holding the endpoint request lane until the command exits.

A new API response and named JSON control may be added without appending variants to any frozen codec. Use the operation registry for the new method. Existing method-shape expectations must remain unchanged.

The client tracks accepted log IDs within the originating endpoint and boot. A failed completion creates an existing-style rejected notice. A completion for a removed float can still report failure, but cannot restore controls or focus. Success clears pending tracking without closing the client. Disconnect or boot change ends tracking as unavailable. Do not replay an action or a completion after reconnect.

Full command output remains in the bounded plugin log and is accessible through the existing ordinary API. This design does not require making `plugin.log.list` available to endpoint clients. Notices remain independent of pane typing and do not disconnect other endpoints.

## Button layout and input

Use one pure layout function for both drawing and hit testing. Inputs are the effective outer rectangle and validated button declarations. Output is the rendered title budget and explicit button rectangles, with no terminal locks, registry access, filesystem reads, or command execution.

Reserve both top corners. Each button occupies its icon width plus one blank cell on each side. Buttons form a right-aligned group inside the top border, with no extra gap between padded rectangles. The title uses the remaining cells on the left and the existing truncation rules.

Controls take priority over title text. If the whole list does not fit, show the largest declaration-order prefix that fits and omit the rest. Never draw or hit-test a partial button. If even the first button cannot fit, draw none and retain the normal border. Omitted controls have no invisible hit regions. With zero configured or negotiated buttons, take the existing drawing and input paths unchanged.

Hit-test floats from top to bottom. Once the topmost float contains the cell, no lower float receives the event, even when the lower button is visible elsewhere. Within that float, buttons take priority over border gestures. Use current displayed geometry, including drag previews, clipping, and title truncation.

A left press on a button captures the full identity tuple and button rectangle. It neither focuses nor raises the float. This avoids changing workspace or tab context merely to activate a control. It does not start a drag or forward mouse input to either terminal.

A matching left release activates exactly once if the same button remains under the pointer with the same binding and layout. Leaving the rectangle cancels permanently for that press, even after re-entry. Also cancel on geometry or stacking changes, clipping, metadata revision change, float disappearance, workspace or endpoint switch, overlay interception, mouse-capture disable, or loss of terminal focus. Consume the captured gesture through release after cancellation so the release cannot reach the pane below.

Right and middle presses, wheel events, and unmatched releases on button cells are consumed without activation, focus, or drag. Buttons do not add a keyboard shortcut in this feature. Existing keyboard interactions remain unchanged.

Represent press state explicitly as idle, armed, or cancelled-until-release. Do not reuse the move/resize drag state for buttons. A pending action response is separate from an active pointer gesture.

## Contracts reserved for T4 and T5

T4 retains existing move/resize rules. Buttons request the `pointer` affordance. A button press never enters the drag-shape state. Mouse-up ends an active gesture override even if its geometry request is pending. Stationary-pointer invalidation and host pointer restoration remain required by invariants 5–6. OSC 22 capability probing and terminal passthrough require separate implementation evidence in T4, not assumptions from this design.

T5 uses the exact float ID from `float_target`, never a workspace lookup followed by toggle. Add a distinct ordinary JSON API method `float.hide-target` with required `boot_id` and `float_id` parameters. It validates the boot and hides that exact existing float in one server turn. An already-hidden target succeeds without mutation. A stale boot or missing target returns an error without creating or reopening a float. This method does not change the published `float.hide` parameter shape.

The hide script reads the validated target from its invocation context and calls `float.hide-target`. It never calls open, show, toggle, or close. Repeated hide preserves the runtime and geometry. Do not implement an unsafe check-then-act sequence over an unguarded API.

A hidden workspace float remains associated with that workspace. Reopening selects the workspace's current tab shell through tabshell's existing selection mechanism. A delayed action uses the clicked float's stable identity, not whichever tab or workspace is active when its script runs. Live tabshell validation belongs to T5–T6.

## Alternatives rejected

- Extending `endpoint.floats.v1` changes a published bincode codec. Separate optional JSON metadata preserves the old client contract.
- Adding optional target fields to `plugin.action.invoke` allows an old server to ignore the target and report success. A new method fails closed.
- Resolving ownership from titles or the current plugin ID after relink can invoke a different installation. A retained installation identity prevents reassignment.
- Retaining old declarations forever avoids refresh work but leaves removed actions visible. Revisioned reload adoption makes stale clicks reject explicitly.
- Activating on mouse-down prevents release cancellation. Release activation supports cancellation without triggering float movement.

## Verification matrix

These are required future tests and live checks, not tests executed by T1.

| Invariant | Required proof | Implementation task |
| --- | --- | --- |
| 1 | Click two plugins with identical action IDs and verify exact plugin, float, workspace, entrypoint, and server-built context. Reject wrong targets, old revisions, relinks, disabled plugins, and removed floats without spawning commands. Preserve bindings across supported live handoff. | T2, T3, T6 |
| 2 | Parse zero, one, and multiple buttons. Reject duplicate IDs, unknown actions, malformed icons, standalone combining marks, controls, format controls, selectors, and joiners. Accept one-cell, two-cell, and base-plus-combining clusters. Check actionable errors. | T2, T6 |
| 3 | Characterize existing no-button rendering, focus, move, resize, and terminal mouse routing. Omitted declarations, empty declarations, and unsupported controls produce the existing behavior. | T2, T3, T4, T6 |
| 7 | Assert the same layout rectangles drive drawing and hit tests. Cover exact fit, first-button overflow, omitted suffixes, truncated titles, clipped geometry, two-cell icons, padding clicks, and stacked overlap. Test cancellation, non-left input, disappearing controls, and release consumption. | T3, T6 |
| 8 | Keep frozen codec bytes, digests, and existing method shapes unchanged. Test old/new client-server combinations, omitted hello fields, each partial-support case, missing metadata, stale boots, endpoint replacement with reused float IDs, and metadata clearing. | T2, T6 |
| 9 | Exercise dispatch rejection, spawn failure, concurrency limit, nonzero exit, timeout, disconnect, and late completion. Verify notices, no implicit retries, no alternate actions, and continued input/rendering on other endpoints. | T2, T3, T6 |

Invariants 4–6 also constrain later work. T5–T6 must prove repeated hide, stable float identity and shell process, retained geometry, and current-tab selection after hidden tab switches. T4–T6 must prove stationary-pointer updates, drag override lifetime, shutdown restoration, unsupported-terminal usability, and probe reply isolation.

Use existing pure state constructors and identity invariants for ownership and handoff tests. Use `just test-one <filter>` during implementation and `just check` before an agreed commit. Preserve existing endpoint fixtures. Add new fixtures only for newly named contracts. Run the project's render-scaling checks if implementation adds work to pane-scaled layout or rendering loops.

## T1 validation and remaining work

T1 validation checks this document against the source boundaries listed above, checks that every T1 invariant has a verification row, and checks that only the T1 status changes in the spec. `git diff --check` validates tracked whitespace. The ignored local document also requires an explicit whitespace check.

No protocol or manifest behavior is implemented by this document. T2 must turn these proposed JSON shapes into schema-backed contracts and tests before exposing them. T4 must establish terminal-specific OSC 22 behavior. T5 must inspect the actual tabshell source and wire its close action to `float.hide-target`. Completion of T1 does not mark those tasks complete.

## T2 as built

Where the implementation differs from or adds detail to the contracts above:

- Button validation is shared, not duplicated. `src/protocol/endpoint.rs::valid_button_icon` is the single icon rule; `src/app/api/plugins/manifest.rs::validate_pane_buttons` applies it with the id grammar, per-pane uniqueness, and the same-plugin action reference before a plugin becomes available, and the client applies the same function to every metadata icon. Format-character and combining-mark checks need `unicode-general-category`; graphemes and widths still come from `unicode-segmentation` and `unicode-width`.
- Installation identity and boot identity use `uuid`: `InstalledPluginInfo.installation_id` is minted when a plugin is first linked or first opened as a float, and preserved across reload and update while `manifest_path` is unchanged; `AppState.server_boot_id` is minted per server boot.
- The binding lives in `src/app/api/plugins/float_controls.rs`. `open_plugin_float` creates it, `revise_float_plugin_bindings` gives affected floats a fresh `controls_revision` after a reload, and `reconcile_float_plugin_bindings` clears `controls` (leaving the terminal and identity intact) when the installation, entrypoint, action platform, or manifest availability no longer resolves.
- Metadata is server-side (`HeadlessServer::prepare_float_buttons`) and is only prepared for a client that sent `float_buttons` together with `floats`. It is regenerated when the presented float set changes or `float_controls_generation` advances, and it is dropped when the client's float interest is cleared.
- Completion is routed from the existing `PluginCommandFinished` event to clients with a pending log id, so the endpoint request lane is never held for the command's lifetime. The client reports a failed completion as an endpoint rejected notice and drops its pending entry; it never retries.
- `App::handle_float_button_invoke` performs every identity check in the same server turn that resolves the action, and derives workspace and tab context from the float's own workspace. For a shared float it leaves workspace and tab context absent rather than substituting the active workspace.

At the time of T2, T3 through T6 remained open. T3 (rendering and hit testing), T4 (pointer shapes), and T5 (the tabshell close button) have since landed. See their sections below.

## T4 as built

- **Negotiation.** The rendered client adds `OSC 22 ; ?pointer,move,ew-resize,ns-resize,nwse-resize ST` to its startup capability query, before DA1. `src/client/host_pointer.rs::HostPointerSupport::from_reply` accepts only an answer with exactly one `0` or `1` per name, the shape kitty's `window.py` sends. `consume_host_keyboard_probe_responses` drains that reply and counts it only when it arrives before DA1. Other OSC 22 strings stay as input. Direct attach and Windows never ask, so they never write OSC 22. A host that does not answer is unsupported, and Herdr writes no OSC 22 to it. A reply that arrives after the probe timeout is dropped by the input framer, whole or split by an idle gap, so it never reaches a pane.
- **Push and pop only.** `HostPointer::sync` pushes (`>shape`) when Herdr starts overriding the pointer. It pops and pushes when the shape changes, and pops (`<`) when the override ends. It writes only shapes the host confirmed. Herdr never reads or replays the host's previous shape: the host's own stack restores it. Herdr also does not project the OSC 22 requests of pane applications.
- **Desired shape.** `ClientShellState::host_pointer_shape` is pure:
  - An active `Dragging` gesture keeps its gesture shape, even off the border.
  - An armed button press shows `pointer`; a cancelled one shows nothing.
  - A button held by any other gesture shows nothing.
  - Otherwise the topmost float under the last reported pointer cell decides. A button shows `pointer` and content shows nothing. Borders use `ClientFloatGesture::for_border`, so the shape is always the gesture a press would start.
  - `floats_own_point` is shared by mouse routing and the pointer. Overlays, the modal popup, and the notification toast suppress the override.
  - Mouse-up ends the gesture override even while the commit is pending (`Committing`).
- **Recompute and restore.** The client loop calls `shell_runtime::sync_host_pointer` once before every wait. It therefore runs after every input, layer, metadata, endpoint, overlay, or config change, including when the pointer does not move. Without host mouse capture, or without a shell, there is no override. A frozen presentation skips the sync. `TerminalGuard` owns the applied shape. Restore, drop, and panic pop it **before** leaving the alternate screen, because kitty keeps one stack per screen. A host that resets itself empties its own stacks; the later pop then has no effect.
- **Live evidence.** A pty harness ran the debug client against a fake kitty-protocol host and a silent host. The harness used an isolated `HOME`, a float plugin, and SGR motion over every cell.
  - Fake kitty-protocol host:
    - The title row gives `move`. Body rows give `move` at the left border and `ew-resize` at the right. The bottom row gives `ns-resize`, then `nwse-resize` at the corner.
    - A resize drag that leaves the border writes nothing until release.
    - On detach, the pop came before `CSI ? 1049 l`.
  - Silent host (the tmux and nested-Herdr case): 82 cell transitions produced zero OSC 22 bytes, the float still opened, and detach worked.
  - An interior cell outside the pane frame (the frame is one column narrower than the interior) shows `move`, because a press there already starts a move.
  - No real kitty window was driven: driving GUI apps on the host is off limits. Kitty's reply format comes from its source (`kitty/window.py`, `kitty/screen.c`). A pixel-level check in real kitty belongs to T6.

## T5 as built

- **Method.** The method is named `float.hide_target`, not `float.hide-target`, to match the underscore style of `float.set_hide_rule`. Its `FloatHideTargetParams` has required `boot_id` and `float_id` fields and rejects unknown fields. `App::handle_float_hide_target` rejects a different boot with `stale_boot`, then reuses the `float.hide` path. `edit_float` reports an already-hidden float as unchanged, so a repeat neither re-renders nor resizes it. A missing or closed float returns `float_not_found`. The method is `changes_ui` but not `client_shell`, so the endpoint method set and frozen codecs do not change. `float.hide` keeps its shape.
- **Tabshell.** The manifest adds a `hide` action and gives the `shell` pane `buttons = [{ id = "close", icon = "x", action = "hide" }]`. The icon is ASCII because `×` (U+00D7) has East Asian Ambiguous width and may render two cells wide, out of step with the hit test. `herdr-tabshell hide` reads `float_target.boot_id` and `float_target.float_id` from `HERDR_PLUGIN_CONTEXT_JSON` and makes one `float.hide_target` call. It does not read `HERDR_FLOAT_ID`, which a float's own processes also inherit. Without a `float_target`, for example from the palette or `plugin.action.invoke`, it exits nonzero without an API call. It takes no lock, touches no tmux state, and never calls `float.show`, `plugin.pane.open`, or `float.close`.
- **Reopen.** This uses the unchanged `toggle`. Its hidden branch selects the current tab's tmux window, then calls `float.show` on the saved float ID. A reopen after a hidden tab switch therefore shows the same float with the current tab's shell. A herdr without float buttons ignores the `buttons` key; the manifest parser has no `deny_unknown_fields`.
- **Evidence.** Herdr tests cover hide, repeated hide with the whole float record unchanged, stale boot, missing and closed floats, and strict params. Tabshell tests cover the exact single call and refusal without a target. The live click in a real client belongs to T6.

## T6 as built

- **New unit tests.**
  - Icon validation now covers a two-cell emoji, `\n`, `\r\n`, DEL, C1 CSI, BOM, ZWSP, supplementary variation selectors, and ZWJ sequences.
  - Invalid button IDs report the plugin, pane, and button index.
  - A button on a background-workspace float uses that workspace and its current tab, not the active one. A target naming the active workspace, or a shared float, fails with `float_button_target_mismatch`.
  - At the plugin command limit, a button fails with `plugin_command_limit_reached`. A command that cannot start fails only its own button's log.
  - A legacy handoff record without `plugin_binding` claims no plugin.
  - On the client:
    - A server-rejected invoke shows a notice, sends no other request, and never retries.
    - A float that disappears under an armed press consumes the release.
    - Switching to an endpoint that reuses `f1` clears the metadata, the press, pending results, and the pointer override. Late metadata from the old boot never attaches.
- **Existing coverage relied on.**
  - Partial negotiation: `float_button_negotiation_requires_layer_metadata_and_method`.
  - Opt-in and old clients: the server tests send no metadata or invoke lane without `float_buttons`, and the hello field defaults to off.
  - Identical action IDs in two plugins, and reload, disable, re-enable, unlink, and relink: `float_button_dispatch_is_targeted_and_stale_bindings_fail_closed`.
  - Handoff revalidation: `float_plugin_binding_handoff_revalidates_installation`.
  - The T3 and T4 layout, press, and pointer tests.
- **Compatibility.** `git diff 3302e00b..HEAD -- src/protocol/floats.rs tests/fixtures/` is empty.
- **Live tabshell.** A pty harness ran the debug server and client with an isolated `HOME`, XDG tree, and `TMUX_TMPDIR`. It clicked the rendered `x` with SGR mouse input. All 18 checks passed:
  - The click hides the clicked float. The float keeps its ID and geometry, stays hidden, and no float is created.
  - Two clicks in one input batch send two invokes (two `float.hide_target` calls). One float remains, and it stays hidden.
  - After a hidden tab switch, toggle reopens the same float and geometry on the second tab's tmux window. The first tab's shell process survives.
  - In two workspaces, clicking `x` hides only that workspace's float.
  - After `herdr server live-handoff` and a reattach, the float, its geometry, and its shells survive, and the close button still works. Live handoff drops attached clients by design.
  - A host that does not answer the probe receives no OSC 22.
- **Real kitty.** kitty cannot run in the tart VM: it needs OpenGL 3.3, and the virtual GPU has no suitable pixel format. Ghostty 1.3.1 answers DA1 but ignores the OSC 22 query, so it gets no shapes. The check therefore ran in the user's host kitty, with an isolated debug Herdr and a float with two buttons. Every shape matched invariant 5: `move` on the title row, left border, and top-right corner; `ew-resize` and `ns-resize` on the right and bottom borders; a diagonal resize at the bottom-right corner; a pointing hand on the buttons; and the I-beam on content and outside the float. A right-border drag held `ew-resize` off the border until release, and detach restored the I-beam.

## T7 as built

- **Remaining `just test` scripts.** The maintenance, ui-hot-path-architecture, integration-assets, and docs-contract scripts pass. The full nextest run passes (4185 tests), and fmt and clippy are clean.
- **Render scale.** `just bench-render-scale` ran on this branch and on `feat/float-shadow-style`, one after the other on an idle machine. Across 84 matching rows, the median new/old time is 0.994 and the maximum is 1.049. There is no regression.
- **Mixed versions.** The "old" binary is the `feat/float-shadow-style` build: it has the float layer but no button metadata or invoke method. In both pairs, the float renders without buttons, a title drag moves it, typing reaches it, and no button action runs. With a new server and an old client, a second, new client on the same server does draw the button. An old server accepts the tabshell manifest, ignores its `buttons`, and rejects `float.button.invoke` as an unknown method.
- **Failure paths, end to end through the client.** A test plugin gives one float four buttons.
  - A late failure (exit 3 after 4 s) shows "Float action failed" with the action's stderr. Typing works before and after it, and the float stays shown.
  - A hung action runs once and never retries. The client stays usable. When the hung command is killed, a failure notice appears.
  - Herdr has no plugin command timeout, and local endpoints have no heartbeat. The only clock is the client's 60 s request timeout. A server stalled with SIGSTOP across a click shows "Server timed out". After the server resumes, the late response arrives, and the action runs at most once.
  - A client killed while an action runs leaves exactly one run. The next client sees no replayed notice.
  - No failure ever triggers another button's action.
- **Typing after reopen and after live handoff.** After reopen and after handoff, typed commands reach the float's tmux shell. A newly attached client, with or without handoff, starts with no float focused. This was already true before this branch. The check therefore clicks the float before typing.
- **Open finding: quarantine after a timeout.** A timed-out request quarantines the client until the late response settles and a newer snapshot revision arrives. `float.button.invoke` changes no snapshot, so after a late button response the client keeps blocking new actions ("Waiting for the previous action") until an unrelated state change or a reconnect. Invariant 9 still holds. A fix would let a settled `float.button.invoke` clear the quarantine without a newer snapshot. That changes when the client may follow an uncertain action, so it waits for a spec decision.
