# neru hints over herdr

Status: research note. Nothing here is implemented in herdr. Sections 1–4 come from reading source; the PoC and calibration sections at the end record measured runs. It answers whether herdr can support [neru](https://github.com/y3owk1n/neru)'s hints mode. Hints mode puts a short label on each clickable element, and typing the label clicks that element.

Sources pinned:

- herdr: this checkout, branch `local`, `2b519523`.
- neru: `y3owk1n/neru@e108948aa933ef83b47037e8bd84484bbd72fb43` (main, 2026-11-13; latest release v1.62.0). Below, `neru:` links point to blobs at that SHA.
- kitty: local fork `~/Projects/Opensources/kitty`, branch `local`, `fc459c4df`.
- Ghostty: `ghostty-org/ghostty@cf3a8a6594eec899fe6335465668c60f2281ce9c`.

All facts below come from reading source code. Nothing was built or run. Behavior that only comes from reading code is marked **inferred**. Behavior that was not checked is marked **unverified**.

## Verdict

- **neru's default hints (`axtree`) cannot see herdr's UI.** herdr draws its sidebar, tabs and float buttons as cells inside the host terminal. kitty and Ghostty each expose that whole surface to Accessibility (AX) as a single `AXTextArea` with no children. A program running inside the terminal has no way to add AX elements.
- **neru already has the extension point that solves this.** `neru hints --targets` (shipped in v1.58.0) labels any rectangles another tool gives it. When a label is chosen, neru clicks at the target's coordinates with a synthesized mouse event. kitty passes that click to herdr, and herdr's existing mouse handler acts on it. So no new click logic is needed on either side.
- **The one open problem is converting coordinates.** herdr knows its hit rects in terminal cells. neru needs global screen points. herdr does not know where its grid's top-left corner is on screen.
- **Recommendation:**
  1. Build approach B: a herdr client action that writes the client's `ShellHitMap` out as `--targets` JSON and pipes it to `neru hints --targets -`. First prototype it with a fixed calibration offset for kitty.
  2. If that conversion proves fragile (padding, scale, kitty splits, multiple monitors), build approach C: herdr's own in-terminal hint mode. It needs no conversion, works over SSH and in any host terminal, and costs more code.
  3. Approach D (neru `contour`/`segment` strategy, or grid mode) works today with no code changes. Use it as the stopgap.

## 1. How neru finds and activates hint targets

### Element discovery (default `axtree` strategy, macOS)

1. **Which windows it scans.** It scans the frontmost window plus any popovers over it. If that comes back empty, it scans only the frontmost window.
   - `internal/adapter/accessibility/collector.go::windowsToScan` ([L209-227](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/accessibility/collector.go#L209-L227)) calls `NeruGetFrontmostAndPopoverWindows`.
   - `NeruGetFrontmostAndPopoverWindows` reads `kAXFocusedWindowAttribute` and `kAXWindowsAttribute` from the focused app in one `AXUIElementCopyMultipleAttributeValues` call. It keeps windows whose `kAXRoleAttribute` is `AXPopover` ([accessibility_window_darwin.m L86-215](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/accessibility_window_darwin.m#L86-L215), popover check at L209).
   - The focused app comes from `kAXFocusedApplicationAttribute` on `AXUIElementCreateSystemWide()` ([accessibility_element_darwin.m L19-50](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/accessibility_element_darwin.m#L19-L50)).
2. **How it walks the tree.** Each window is walked by `native.Client.ClickableNodes`, then `BuildTree`, then `TreeNode.FindClickableElements` ([native/client.go L90-118](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/accessibility/native/client.go#L90-L118), [native/darwin/tree.go L691-718](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/accessibility/native/darwin/tree.go#L691-L718)).
   - Children come from `kAXChildrenAttribute` (`NeruGetChildren`, [L565-605](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/accessibility_element_darwin.m#L565-L605)).
   - Tables and outlines use `kAXVisibleRowsAttribute` (`NeruGetVisibleRows`, [L608-645](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/accessibility_element_darwin.m#L608-L645)).
   - Depth is capped by `hints.max_depth`, default 50 ([configuration.md L586](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/docs/reference/configuration.md#L585)).
3. **Attributes it reads.** Each node's attributes are fetched in one batch by `NeruGetElementInfo` ([L276-316](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/accessibility_element_darwin.m#L276-L316)):
   - `kAXPositionAttribute`, `kAXSizeAttribute`
   - `kAXTitleAttribute`, `kAXDescriptionAttribute`, `kAXValueAttribute`, `kAXIdentifierAttribute`
   - `kAXRoleAttribute`, `kAXSubroleAttribute`, `kAXRoleDescriptionAttribute`
   - `kAXEnabledAttribute`, `kAXFocusedAttribute`, `kAXHiddenAttribute`, and `AXVisible`

   neru does not read an `AXFrame` attribute. The frame is computed from position and size.
4. **Clickability filter.**
   - **Role.** The node's role must be in `hints.clickable_roles`. The default set is button, menu/popup button, combo box, checkbox, radio, switch, disclosure, link, text field, text area, slider, tab, menu item, cell, row, list item, and `ax:AXGenericElement` ([element/vocabulary.go L323-347](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/domain/element/vocabulary.go#L323-L347)). The semantic `text_area` maps to `AXTextArea` ([L206](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/domain/element/vocabulary.go#L206)).
   - **State, action and visibility.** `NeruHasClickAction` ([L789-830](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/accessibility_element_darwin.m#L789-L830)) applies these rules:
     - It drops hidden or invisible nodes.
     - An enabled node that is not a container passes.
     - Disabled nodes and containers (`AXScrollArea`, `AXGroup`, `AXSplitGroup`) are judged by `AXUIElementCopyActionNames`. The click actions it looks for are `AXPress`, `AXShowMenu`, `AXConfirm`, `AXPick` and `AXRaise` (`hasClickActionName`, [L754-776](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/accessibility_element_darwin.m#L754-L776)).
     - An optional hit test, `NeruIsElementVisibleAtPoint` ([L691-712](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/accessibility_element_darwin.m#L691-L712)), calls `AXUIElementCopyElementAtPosition` at the node's center and checks that the element found there is the node or one of its ancestors.
   - **Size.** Nodes with zero width or height are dropped ([tree.go L707-710](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/accessibility/native/darwin/tree.go#L707-L710)).
5. **Supplementary sources.** Each one is opt-in through `include_*` flags: menubar, Dock, Notification Center, Stage Manager, Picture in Picture, and Screen Capture.
   - Example: the Dock is read from the Dock app's `kAXChildrenAttribute` with the extra role `AXDockItem` ([adapter_supplementary.go L135-151](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/accessibility/adapter_supplementary.go#L135-L151), [accessibility_screen_darwin.m L200-235](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/accessibility_screen_darwin.m#L200-L235)).

### Per-app config

`[[hints.app_configs]]` entries are keyed by `bundle_id` ([config.go L258-271](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/config/config.go#L258-L271), [configuration.md L874-889](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/docs/reference/configuration.md#L874-L889)). Each entry can override:

- `strategy`
- `capture_scope`
- `additional_clickable_roles`
- `ignore_clickable_check`, which skips the press-action, hidden and hit tests
- `visible_check_enabled`
- `hotkeys`
- `select_with`

### Non-AX strategies

`hints.strategy` takes one of four values: `axtree` (the default), `vision`, `contour` or `segment` ([domain_constants.go L96-119](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/domain/domain_constants.go#L96-L119), [configuration.md L545-561](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/docs/reference/configuration.md#L545-L561)). All three non-default strategies work from a screen capture (ScreenCaptureKit `SCScreenshotManager`, [vision_darwin.m L250](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/vision_darwin.m#L250)).

- **`vision`** runs Apple Vision requests on the capture: `VNRecognizeTextRequest` (OCR), `VNDetectRectanglesRequest` and saliency. A classifier then assigns roles to what they find ([vision/doc.go L1-28](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/vision/doc.go#L1-L28), [vision_darwin.m L382, L398](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/vision_darwin.m#L382-L398)).
- **`contour`** is wl-kbptr's edge detector, written in pure Go.
- **`segment`** splits the capture into controls, pictures and words. With `hints.segment.targets = "words" | "phrases" | "controls"` it hints every word, phrases, or only controls ([configuration.md L805-811](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/docs/reference/configuration.md#L805-L811)).
- **Grid modes** are separate from hints: `grid`, `recursive_grid` and `bisect` ([configuration.md L891](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/docs/reference/configuration.md#L891), L964). They need no element discovery.

### External targets: `neru hints --targets`

`neru hints --targets` was added in [PR #1770](https://github.com/y3owk1n/neru/pull/1770) (merged 2026-10-12) and released in v1.58.0 ([CHANGELOG.md L136-149](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/CHANGELOG.md#L136-L149)). Contract ([cli.md L470-474, L535-560](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/docs/reference/cli.md#L535-L560), [scripting.md L147-159](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/docs/guide/scripting.md#L147-L159)):

- The input JSON has this shape: `{"hints":[{"x":…,"y":…,"width":…,"height":…,"title":"…"}]}`.
  - `x`, `y`, `width` and `height` are required. They use the coordinates `move_mouse` takes: global, top-left origin, Y grows downward.
  - `role`, `title`, `description` and `value` are optional.
- `--targets -` reads the JSON from stdin. Over the IPC socket, the JSON travels as one argument: `--targets={...}` ([ipc.md L139](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/docs/reference/ipc.md#L139)).
- neru drops any target whose center is not on the active display.
- It refuses `--targets` together with `--role`, `--text`, `--strategy`, `--capture-scope`, `--split-word` or `--repeat`.
- A screen change exits the mode.
- The IPC request size cap holds "several hundred" targets.

`neru query window --json` returns the focused window's bounds in the same coordinate space ([cli.md L168-240](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/docs/reference/cli.md#L168-L240)).

### Activation: synthesized mouse click, not `AXPress`

- For scanned AX elements too, `Adapter.PerformAction` clicks at `element.Center()` ([adapter.go L158-181](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/accessibility/adapter.go#L158-L181)).
- `native.Client.PerformAction` then calls `LeftClickAtPoint` ([native/client.go L252-275](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/accessibility/native/client.go#L252-L275)).
- That posts `CGEventCreateMouseEvent` events: `kCGEventLeftMouseDown` and `kCGEventLeftMouseUp` with click-state counting ([accessibility_mouse_darwin.m L240-260](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/adapter/platform/darwin/accessibility_mouse_darwin.m#L240-L260)).
- Targets from vision, contour, segment and `--targets` have no AX handle, so they can only be clicked by coordinates ([element.go L240-245](https://github.com/y3owk1n/neru/blob/e108948aa933ef83b47037e8bd84484bbd72fb43/internal/domain/element/element.go#L240-L245)).
- `AXPress` is used only as a *filter signal* (step 4 above). No call to `AXUIElementPerformAction` was found in the darwin adapter.

### Open issues

`gh search issues --repo y3owk1n/neru` for terminal, kitty, ghostty, tmux, wezterm, alacritty, targets, external, and "custom hint" found no issue about terminals or TUI apps. PR #1770 names canvas apps, games and remote desktops as its motivation.

## 2. Why herdr gets no useful hints today

- **Where herdr's UI lives.** herdr is a TUI client process inside a host terminal. The README calls it "one rust binary, no electron — runs in whatever terminal you already use" (`README.md:38`). The client turns on host mouse reporting with crossterm `EnableMouseCapture`, plus SGR pixel mode `?1016h` when the host supports it (`src/client/terminal_setup.rs:670-690`). So the macOS process that owns herdr's pixels and AX tree is the host terminal (kitty, Ghostty, …), not herdr.
- **kitty's AX surface.** kitty's `GLFWContentView` returns `isAccessibilityElement = YES` and role `NSAccessibilityTextAreaRole`. It allows only these selectors:
  - text: value, selected text and range, string for range, number of characters, insertion line
  - window geometry: window, parent, position, size

  `accessibilityChildren` is not on the list (`kitty/glfw/cocoa_window.m:2339-2365`).
- **Ghostty's AX surface.** Ghostty's `SurfaceView` is also one `.textArea` element. Its value is the cached screen text ([SurfaceView_AppKit.swift L2353-2370](https://github.com/ghostty-org/ghostty/blob/cf3a8a6594eec899fe6335465668c60f2281ce9c/macos/Sources/Ghostty/Surface%20View/SurfaceView_AppKit.swift#L2353-L2370)). No children are exposed.
- **Result.** neru's tree walk reaches one `AXTextArea` per terminal surface. `text_area` is a default clickable role, so the **inferred** outcome is one hint covering the whole terminal (plus the terminal's own window chrome or tab bar, if any). herdr's sidebar rows, tabs, `+` buttons and float buttons get no hints.
- **Unverified:** what other host terminals (WezTerm, iTerm2, Terminal.app) expose. None of them is checked out here.

## 3. herdr's clickable UI and existing interfaces

### Hit map

All shell chrome is hit-tested against one per-render map, `ShellHitMap` (`src/client/shell/state.rs:107-164`). Its entries are in terminal cells (ratatui `Rect`):

- sidebar: `machines`, `workspaces` (`WorkspaceHit` with an optional `group_toggle`, `state.rs:279-285`), `agents`, `endpoint_agents`, `agent_sort_toggle`, `sidebar_toggle`, `sidebar_divider`, `new_workspace`, `global_launcher`
- tab bar: `tabs` (`(Rect, tab_id)`), `new_tab`, `tab_scroll_left`, `tab_scroll_right`
- panes and splits: `panes` (`PaneHit`, `state.rs:170-181`), `pane_splits`
- floats: `floats` (`FloatHit`), each with title-bar `buttons: Vec<(id, Rect)>` laid out from plugin manifests (`src/client/shell/floats.rs:84-91`)
- overlays: menus and popups (`global_menu_rows`, `context_menu_rows`, `navigator_rows`, `worktree_rows`, `settings_tabs`, `settings_choices`, `overlay_primary`, `overlay_cancel`, …), `notification_toast`, and the mobile layout targets

How the map is built and used:

- `render::render_shell` builds a fresh map on every frame (`src/client/shell/render.rs:253-260`). The result is stored at `src/client/shell/composition.rs:217`.
- The map lives in the **client** and is `pub(super)`. Each attached client has its own map.
- Mouse dispatch goes through `ClientShell::handle_mouse` (`src/client/shell/mouse.rs:679-…`). It checks overlays first, then shell controls, then panes, using `contains(self.hits.<field>, (col,row))` (`src/client/shell.rs:135`).
- A left-button `MouseEvent` Down/Up at a rect's center therefore triggers the same action a real click does. This holds whether the click comes from neru through the host terminal or is synthesized inside herdr.

### Socket API

The socket API is served by the herdr server (`src/api/mod.rs:35`, `src/api/server.rs`). It is documented at `docs/next/api/herdr-api.schema.json`.

- Its only geometry is `PaneLayoutSnapshot`: pane and split rects in cells (`src/api/schema/panes.rs:678-709`). The read query `pane.layout` returns it (`src/api/schema.rs:100`, `src/cli/pane.rs:263`), and so do `pane.neighbor`, `pane.edges` and several mutation results.
- It has no sidebar, tab, float-button or overlay geometry. Those exist only in the client's hit map.
- **Unverified:** which origin `PaneLayoutSnapshot.area` uses compared with a given client's full terminal grid, given multiple clients and sidebar width.

### Plugins

Plugins declare panes and float buttons through manifests and run commands with `HERDR_PLUGIN_CONTEXT_JSON` (see `docs/design/float-controls.md`). No plugin hook reads client-side geometry.

### Cell size

The client knows the grid size and the cell size in pixels from the `TIOCGWINSZ` pixel extent (`src/client/terminal_geometry.rs:17-42`). Its fallback is 8×16. It does not know any of these:

- where its grid starts on screen
- the host's title bar, tab bar or padding
- kitty's internal window splits
- the backing scale factor

**Unverified:** whether kitty reports the ioctl pixel size in device pixels or in points on Retina displays.

## 4. Approaches, ranked

### B. herdr emits `--targets` JSON to neru (recommended first)

What changes:

- **herdr client.** Add an action, for example `export_hint_targets` or `neru_hints`, that runs in `ClientShell` (wire it next to the other client actions in `src/client/shell/actions.rs` / `input.rs`). It does three things:
  1. Walk `self.hits` and collect the clickable rects with titles: workspace names, tab titles, button names, float button ids, pane bodies.
  2. Convert each rect: `screen_x = origin_x + col * cell_w_pt`, `screen_y = origin_y + row * cell_h_pt`.
  3. Run `neru hints --targets - --action left_click` and write the JSON to its stdin.

  Labels can use the strings herdr already has, so neru's `/` search works on them.
- **Coordinate origin.** This is the blocker. Options, cheapest first:
  - a. A config value for the kitty content offset. Combine it with `neru query window --json` (window frame) and the cell pitch divided by the backing scale. It breaks with kitty splits, tab bars and changed padding.
  - b. Ask kitty over its remote-control protocol (`kitty @ ls` gives per-window geometry). **Unverified:** whether that reports screen-space pixel origins.
  - c. Calibration: a one-shot click on a known cell. herdr reads its `(col,row)`, and neru supplies the screen point through `neru query cursor`.
- **neru.** No changes. When a label is chosen, neru posts a real click. kitty sends it to herdr as SGR mouse input, and `handle_mouse` acts on it.

Constraints:

- It only works for a local client on macOS (or another platform where neru runs) attached to a GUI host terminal. It does nothing over SSH from another machine's terminal.
- A frame redraw between export and click can move targets. neru exits hints mode on a screen change but not on a TUI redraw. A herdr redraw is not a screen change in that sense (**inferred**).
- Effort: small to medium. Each step is a few hundred lines in the client, plus calibration work.

### C. herdr-native hint mode (robust alternative)

What changes, all in the herdr client:

1. Add a `ClientShellMode::Hint` variant (`src/client/shell/state.rs:316-322`) or an overlay kind.
2. On entry, snapshot `self.hits` and assign vimium-style labels. Reuse neru's idea: short labels first, from a configurable alphabet.
3. Draw label badges over each rect's top-left cell in the render pass that already produces the hit map (`src/client/shell/render.rs`).
4. On key input, narrow the candidates. When one target is left, synthesize `MouseEvent { kind: Down(Left), column, row }` and then `Up` at the rect's center, and call `handle_mouse` (`src/client/shell/mouse.rs:683`). Every existing action is reused, including drag-safe paths, float buttons and overlay rows.
5. Add a keybinding in the prefix table and a settings entry.

Trade-offs:

- No coordinate conversion. Works in every host terminal, over SSH, and on Linux.
- It labels herdr chrome only. Clickable text inside panes (links, agent UIs) would need detection on the pane's content (`src/client/shell/link_hover.rs` already finds links).
- Effort: medium. A new mode, rendering, input handling and tests.
- herdr already has keyboard navigation for workspaces and panes (`ClientShellMode::Navigate`, the navigator overlay `ClientNavigatorTarget` at `state.rs:382-395`). Hint mode adds direct targeting of everything else: tabs, `+` buttons, float buttons and menu rows.

### D. neru screen-capture strategies or grid mode (stopgap, zero code)

Add this to the neru config:

```toml
[[hints.app_configs]]
bundle_id = "net.kovidgoyal.kitty"   # kitty/setup.py:2009 with appname 'kitty' (kitty/kitty/constants.py:23)
strategy = "segment"                  # or "contour" / "vision"
```

`segment` with `targets = "phrases"` or `"words"` should label sidebar entries and tab titles as text. It will also label every word in pane output, so expect noisy hints in busy panes. `contour` finds boxed shapes, which suits herdr's bordered floats and buttons. The click goes through the same mouse path as in approach B.

- **Unverified:** hint quality on herdr's UI. Not run.
- Grid, recursive_grid and bisect always work, but they are spatial, not semantic.
- Effort: none.

### A. Expose herdr regions through the host terminal's AX tree (not feasible)

This would need every host terminal to:

- publish child AX elements for its `AXTextArea`, which kitty and Ghostty do not do (section 2)
- define a new escape-sequence protocol that lets the program inside the terminal declare regions

No such protocol exists in kitty or Ghostty. It would mean patching every host terminal, upstream acceptance is unlikely, and herdr gains nothing that `--targets` does not already give. Rejected.

### Alternative: a neru-side provider plugin

This is not needed. `--targets` (PR #1770) is the provider interface. neru has no other mechanism for plugins or custom hint sources. Its IPC socket and Go client (`client/` package) take the same `--targets` payload.

## Next steps

1. Prototype B outside herdr first. A shell script reads `herdr pane layout --pane <id>` (`src/cli/pane.rs:260-264`) and `neru query window --json`, applies a fixed offset, and pipes to `neru hints --targets -` for the pane rects only. Measure the offset error in kitty with and without kitty splits.
2. If the offset holds, add the client action that exports the full `ShellHitMap`.
3. If it does not hold, build approach C.

## PoC results (2026-11-14, tart VM `mac_jr`)

Setup: macOS 27.2 VM, one 1024×768 display at scale 1. Neru v1.62.0 with Accessibility and Screen Recording granted, herdr 0.9.5 in **Ghostty** (the local kitty build loads Python from the host's pyenv and does not start in the VM). herdr showed the sidebar, three tabs, `+`, and two panes. Each strategy was run with `neru hints --strategy <s>` and listed with `neru query hints --strategy <s>`. Screenshots are in `poc/`.

| Strategy | Targets | herdr controls labelled |
| --- | --- | --- |
| `axtree` | 4 | None. Ghostty's three window buttons plus one hint at the centre of the whole terminal surface (`poc/h-axtree.png`). Confirms section 2. |
| `segment` | 27 | Tabs 1, 2, 3, `+`; sidebar `spaces`, `~` workspace row, `new`, `menu` (split into two hints), `agents`, `grouped`, `«` collapse. Noise: words of each pane's shell prompt and window chrome (`poc/h-segment.png`). |
| `contour` | 13 | Only the active tab and a few border shapes. Misses inactive tabs and the sidebar words. |
| `vision` | 11 | Sidebar words via OCR, but misread ("8gents", "gpouped") and the tabs are missed. |

Click path, measured: `neru action move_mouse --x 380 --y 73` then `neru action left_click` at the `segment` target for tab 3 switched herdr from tab 2 to tab 3 (`poc/click.png`). This is the same synthesized-click path a hint selection uses (section 1), so a chosen hint reaches herdr's `handle_mouse` through Ghostty with no herdr change.

Not tested: choosing a hint by typing its label (keystrokes could not be injected over SSH), kitty as the host terminal, a Retina display (scale 2), and busy panes, where `segment` will label every word of output.

Conclusion: approach D with `strategy = "segment"` is a working zero-code path for herdr's chrome. Its cost is noise from pane text. Approaches B and C remain the route to one exact hint per herdr control.

Config used for approach D (Ghostty; use `net.kovidgoyal.kitty` for kitty):

```toml
[[hints.app_configs]]
bundle_id = "com.mitchellh.ghostty"
strategy = "segment"
```

kitty, attempted 2026-11-14: the official kitty 0.50.0 release cannot start in the tart VM ("NSGL: Failed to find a suitable pixel format"; kitty needs OpenGL 3.3, which the VM's virtual GPU does not provide). A host run was attempted but the host screen was locked, so `axtree` returned 0 hints and `segment` could not capture the display. kitty remains untested.

kitty on the host, 2026-11-14 (screen unlocked, local kitty fork, bundle id `space.noomz.kitty`, not `net.kovidgoyal.kitty`): `axtree` returned exactly 1 hint, an `AXTextArea` covering the whole kitty window (10,73 2540x1623), with herdr's rendered text as its value. This confirms section 2 for kitty. `segment` failed with "failed to capture display" because the host Neru has no Screen Recording permission. kitty `segment` remains untested.

kitty `segment` on the host, 2026-11-14 (Screen Recording granted, real herdr session with busy agent panes, 2560-point-wide display): 729 targets with three-letter labels. herdr's tabs, `+`, and the sidebar rows (machines, workspaces, agents, `new`, `menu`, `grouped`) each got a label, but so did nearly every word of pane output and the macOS menu bar and status items, because the capture covered the whole display rather than the kitty window. In a real session this is too noisy to use as is. Untested mitigations: `capture_scope` limited to the focused window, and `hints.segment.targets = "controls"`. This result is the case for approach B or C: herdr knows its ~30 controls exactly and can hand neru only those.

## Calibration measurements (T1, 2026-11-14)

Setup: the host Mac (M4 Max), neru v1.62.0 daemon, kitty (local fork, bundle `space.noomz.kitty`) and Ghostty from `/Applications`. Scale 2 is the MateView in its 2560 × 1707 HiDPI mode. (A run in the 1920 × 1280 HiDPI mode gave the same scale and cell sizes, but its files were overwritten by the scale-1 runs, which reused the labels.) Scale 1 is the MateView in the 1920 × 1280 low-resolution mode (rendered size equal to the looks-like size). Each run starts a fresh host window that runs `poc/probe.py` directly, not inside herdr. Raw results are in `poc/results/<label>.json`.

### What the probe does

1. It turns on any-motion SGR mouse reporting (`?1003h ?1006h`) and focus reporting (`?1004h`).
2. It warps the pointer with `neru action move_mouse` to two points inside `neru query window`, once with cell reports and once with SGR pixel reports (`?1016h`).
3. **Move and observe, pixel variant.** It computes the scale (device pixels per point) from the pixel delta between the two points, and the grid origin from `point − pixel / scale`. The cell pitch is the ioctl extent divided by the grid size, rounded down, as `ioctl_cell_size` does.
4. **Ground truth.** In cell mode it steps the pointer one point at a time until the reported column, then the row, changes. That boundary gives the grid origin to one point.
5. For each origin it clicks the center of three cells (top-left, middle, bottom-right) with `left_click --bare`, and compares the reported cell with the target.
6. It moves the pointer back to where it started.

Safety rule: the probe clicks only after the warp produced a motion report and `neru query cursor` still shows the target point. Otherwise it skips the click, so no click can land outside the probe window.

### Results

Origin error is pixel-method origin minus ground truth, in points. Window offset is grid origin minus the `neru query window` origin. Clicks are hit / miss / skipped.

| Run | Scale | Cell (pt) | Origin error | Window offset | Clicks | Search tries |
| --- | --- | --- | --- | --- | --- | --- |
| kitty, tab bar bottom | 2 | 9 × 21.5 | 0, −0.5 | 10, 9.5 | 6/0/0 | |
| kitty, no tab bar | 2 | 9 × 21.5 | 0, −0.5 | 10, 9.5 | 6/0/0 | |
| kitty, tab bar top | 2 | 9 × 21.5 | 0, −0.5 | 10, 31 | 6/0/0 | |
| kitty, vertical split, fixed points | 2 | | | | none sent | failed cleanly |
| kitty, horizontal split, fixed points | 2 | | | | none sent | failed cleanly |
| kitty, vertical split, search | 2 | 9 × 21.5 | 0, 0 | 10, 370.5 | 6/0/0 | 4 |
| kitty, horizontal split, search | 2 | 9 × 21.5 | 0, 0 | 552, 9.5 | 6/0/0 | 3 |
| Ghostty, default | 2 | 8 × 17 | 0, 0 | 2, 34 | 6/0/0 | |
| Ghostty, titlebar tabs | 2 | 8 × 17 | 0, 0 | 2, 42 | 6/0/0 | |
| Ghostty, padding 20 | 2 | 8 × 17 | 0, 0 | 20, 52 | 6/0/0 | |
| kitty, tab bar bottom | 1 | 9 × 21 | 0, 0 | 10, 7 | 6/0/0 | |
| kitty, no tab bar | 1 | 9 × 21 | 0, 0 | 10, 7 | 6/0/0 | |
| kitty, tab bar top | 1 | 9 × 21 | 0, 0 | 10, 28 | 6/0/0 | |
| kitty, vertical split, search | 1 | 9 × 21 | 0, 0 | 10, 361 | 6/0/0 | 4 |
| kitty, horizontal split, search | 1 | 9 × 21 | 0, 0 | 552, 7 | 6/0/0 | 3 |
| Ghostty, default | 1 | 8 × 17 | 0, 0 | 2, 34 | 4/0/2 | |
| Ghostty, titlebar tabs | 1 | 8 × 17 | 0, 0 | 2, 42 | 4/0/2 | |
| Ghostty, padding 20 | 1 | 8 × 17 | 0, 0 | 20, 52 | 4/0/2 | |

The Ghostty default and titlebar-tabs JSON files at scale 2 were recorded before the probe switched to the integer cell pitch, so their stored `cell_pt` and edge origin are off. Recomputed with the integer pitch, both origin errors are 0, 0, and all their clicks hit. The scale-1 Ghostty skips are the bottom-right cell: the window extended below the 1280-point screen, so the pointer was clamped, and the guard skipped the click.

Untested: Ghostty's native tab strip in the default titlebar style, which appears only with two or more tabs (only `--macos-titlebar-style=tabs` was run); Ghostty splits; kitty layouts other than one vertical and one horizontal split; and a host window on the second display (see finding 12).

### Findings

1. **Move and observe, pixel variant, is accurate.** The origin agrees with ground truth within one device pixel (0.5 pt at scale 2, 0 at scale 1). Every on-screen click, 90 in total, landed on its target cell in both hosts, at both scales, with every tab-bar layout, Ghostty padding, and kitty splits.
2. **Window frame plus fixed offset fails.** The offset changes with the host and its settings: kitty 9.5 → 31 pt when the tab bar moves to the top, Ghostty 34 → 42 → 52 pt with titlebar tabs and padding, and the full split offset (370 pt, 552 pt) in kitty splits. It would need per-terminal config, which the spec rules out.
3. **kitty remote control has no screen geometry.** `kitty @ ls` window entries have no position fields (keys: `columns`, `lines`, `neighbors`, … only). That candidate is dropped.
4. **Scale must be measured.** `neru query displays` reports `scale: 1` for both HiDPI displays. Two warp points give the scale exactly (2.0 or 1.0 in every run). SGR pixel reports and the ioctl extent are both in device pixels.
5. **Cell pitch must be rounded down.** Ghostty's ioctl extent includes leftover padding (2446 px for 152 columns of 16 px). A float division drifted a full column at the far edge. Integer division, which herdr's `ioctl_cell_size` already uses, is exact in every run.
6. **SGR pixel origin is the grid origin.** It is not the window or padding edge: with the integer pitch, the pixel origin equals the edge-search origin, including Ghostty with 20 pt padding.
7. **`?1016l` drops SGR encoding in kitty.** After turning pixel mode off, kitty sends legacy `ESC [ M` reports until `?1006h` is sent again. Restoring mouse state after calibration (§V.8) must re-send the full mode set, not only `?1016l`.
8. **Synthesized warps produce motion reports.** One warp gives one SGR motion report, in both hosts. In cell mode no report is sent when the warp stays in the same cell, so "no report" is ambiguous only for very short moves.
9. **kitty splits need a point search.** The OS window frame covers every kitty window, and only the window under the pointer reports. Two fixed points failed cleanly ("no motion report"). A 3 × 3 grid of points found the probe's split in 3–4 tries.
10. **Interference is detectable.** When a person moves the real pointer during calibration, the warp's first report is followed by more reports, and `neru query cursor` no longer matches the target. Calibration can detect this and fail or retry instead of using a bad origin.
11. **Cost.** One `neru action move_mouse` takes about 7 ms. A calibration is two warps plus a restore. Waiting for the first report, rather than the probe's fixed 400 ms window, keeps it well under 100 ms. Each failed search point costs one report timeout.
12. **Pointer restore works on the main display only.** In every saved run, `neru query cursor` after the run equals the value before it, but every saved run started on the MateView. `neru action move_mouse` does not cross displays: a move to (−800, 1800), inside the built-in display, lands at (0, 1279), the nearest MateView point. This happened whether the pointer started on the MateView or, in an unsaved run, on the built-in display (start (−1373, 2440), restored to (0, 1706)). So herdr cannot restore a pointer that started on another display, and it cannot calibrate a host window that sits on a display neru will not warp to. T4 must detect both cases (target point not equal to `neru query cursor` after the warp) and report them instead of calibrating.
13. **Stale targets are real.** In one vertical-split run, the other kitty window closed mid-run, kitty resized the probe window, and the same screen point then reported row 18 instead of row 1. A layout change between sending and clicking moves the target (§V.5).
14. **New host windows do not take focus** when they are started from a background process (macOS cooperative activation). This affects only the test harness: in the feature, herdr's own window is focused because the user just pressed its key.

### Decision

Use **move and observe, pixel variant**:

1. Read the cell pitch from the ioctl with integer division (existing `ioctl_cell_size`).
2. Turn on any-motion SGR pixel reporting, saving the current mouse modes.
3. Warp to a candidate point inside `neru query window`: the window center first, then a 3 × 3 grid. Stop at the first point that produces a pixel motion report.
4. Warp to a second point a fixed distance away (40 pt diagonal) inside the same terminal window. Compute the scale from the two reports, and the origin from either one.
5. Check `neru query cursor` against each target point. On a mismatch (someone moved the pointer), on no report from any candidate, or on a scale that is not a positive number, fail and report why (§V.4).
6. Move the pointer back and restore the saved mouse modes, re-sending the full set (§V.8). If the start point is on a display neru cannot warp to (finding 12), report that the pointer could not be restored.

Calibrate on every use. It costs tens of milliseconds, and herdr cannot see window moves, so a cached origin can go stale without warning.

## End-to-end verification (T6, 2026-11-14, partial)

Setup: herdr `b52744c8` release build, neru v1.62.0, built-in Retina display only (scale 2). Each host ran a separate test instance against an isolated named session (`XDG_CONFIG_HOME` in a temp dir, `herdr --session neru-t6`), so the user's own server was not touched. `neru.path` pointed at a wrapper that logged each neru call and saved the targets JSON, then ran the real neru. Clicks on targets used `neru action move_mouse` to the target center and `left_click --bare`, the same click neru makes on a label pick. One real label pick per host used a synthetic keystroke into neru's overlay. herdr state was read back with `herdr workspace list` and `herdr tab list` over the test socket.

### Results

| Check | kitty, scale 2 | Ghostty, scale 2 | Invariants |
| --- | --- | --- | --- |
| Workspace row | pass | pass (2 rows) | 1, 2 |
| Tab | pass | pass (2 tabs) | 1, 2 |
| `+` (opens the new-tab name dialog) | pass | pass | 1, 2 |
| Group collapse, then expand | pass | pass | 2, 13 |
| Sidebar toggle `«`, collapse and back | pass | not sent: off-screen, see below | 2, 11 |
| Narrow layout "switch workspace" button (kitty vertical split, 59 columns) | pass | | 1, 2 |
| Menu row | fail before T7 (finding 1). After T7: pass. With the global menu open, `ctrl+b f` sent 5 targets (`menu` plus the 4 rows), the menu stayed open, and a click at the `keybinds` row center opened the keybinds help | not run | 1, 2, 15 |
| Float button | not run: needs a plugin float | not run | 1, 2 |
| Real neru label pick | pass (`KA` → workspace alpha) | pass (`G` → tab t3) | 2 |
| Titles are visible text | pass | pass | 3 |
| Busy session: 17 workspaces, 8 tabs | 29 targets: 14 visible workspace rows of 17, 8 of 8 tabs; scrolled-off rows not sent | 39 targets, whole sidebar visible | 1 |
| Missing neru (`neru.path` to a missing file) | pass: "neru not found at …", no neru call, no zombie child; the other attached client showed nothing | | 6, 7 |
| Pointer moved during calibration (warps every 30 ms) | pass: "Calibration failed: the pointer moved …", neru not started, pointer warped back | | 4, 8, 10 |
| Pane app with any-motion + SGR, then also pixel mode | pass: 0 reports reached the pane during calibration; afterwards SGR cell reports, and SGR pixel reports in pixel mode, never X10 | | 12, 14 |
| kitty horizontal split (herdr in one half) | pass: 3 of 3 clicks hit, calibration took 4 warps | | 2 |
| Redraw between send and pick (workspace switched by the API while labels were up) | pass: the click landed on the empty tab-bar cell now under the label; nothing broke | | 5 |
| Window taller than the display | Ghostty window ran 43 pt below the 1117 pt screen; the `«` row is off-screen and was not sent | | 11 |
| SSH client | not run: localhost has no key login set up | | 7 |
| Scale 1 | not run: the scale-1 display (MateView) is not attached | | 2, 9 |
| Host window on a second display | not run: no second display attached | | 8, 10 |

Calibration costs: 2 warps plus the restore warp when the window center reports, 4 in the split cases. Each neru call took about 220 ms through the logging wrapper, so a full run took about 2 s; without the wrapper's Python timestamp it is shorter.

### Findings

1. **Menu rows cannot become targets from the keybinding.** While any overlay is open, `ClientShell::route_key_press` (`src/client/shell/input.rs:653`) sends every key to `route_overlay_key`, so `prefix+f` never reaches the prefix table. Design note section 2 makes an open overlay's rows the only targets, but no key press can start the action while it is open. With the menu open, `ctrl+b f` started no neru call. Text-input overlays such as the new-tab dialog take the keys as text. Fixed in T7 (§V.15): a `ContextMenu` or `GlobalMenu` lets `keys.neru_hints` through before `route_overlay_key`.
2. **`[neru]` is not live-reloaded.** `herdr server reload-config` and the client's reload apply `[keys]` and `[ui]` (`ClientShellConfig::apply_live_config`), not `neru.path`. A changed path takes effect only in a new client. Design note section 7 does not say which is intended.
3. **Collapsed-sidebar titles are terse and repeat.** With the sidebar collapsed, the visible text of workspace rows is a number and a dot, so titles are `1·`, `2·`, `▾L`, and `1·` appears three times. This meets §V.3 as written, but neru's search mode cannot tell those targets apart.
4. **Each new client opens on the onboarding overlay** in a fresh config dir, and the overlay takes `prefix+f` (finding 1). This affects only first runs.
