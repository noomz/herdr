# Float control contracts

Status: T1 design for `SPEC-float-controls.md`. These contracts describe proposed behavior, not implemented features.

## Scope and existing boundaries

Herdr owns float borders and input. Plugins declare ordered buttons that invoke their own actions. The server owns bindings, validation, and command execution. The client owns button layout, hover, and press state.

The current boundaries are:

- `src/app/api/plugins/manifest.rs` parses YAML pane declarations and validates action references for link handlers.
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

```yaml
panes:
  - id: shell
    title: Shell
    placement: float
    command: [./shell]
    buttons:
      - id: inspect
        icon: "?"
        action: inspect-shell
      - id: close
        icon: "×"
        action: hide-shell
actions:
  - id: inspect-shell
    title: Inspect shell
    contexts: [workspace]
    command: [./inspect-shell]
  - id: hide-shell
    title: Hide shell
    contexts: [workspace]
    command: [./hide-shell]
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
