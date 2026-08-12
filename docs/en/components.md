# Component Architecture (`engine::ui`)

## 1. Overview

`engine::ui` is the UI component layer responsible for rendering, layout, and
input handling of HTML **replaced elements** — `<button>` / `<img>` / `<input>`.

Ordinary elements are turned into `InfoNode` / `LayoutNode` by `engine::layouter`
and into `DrawCommand`s by `engine::renderer_model`. Replaced elements, however,
delegate rendering to external implementations (native UI, image decoders, IME-aware
widgets), so they connect to the layout engine through an extension point:
the `CustomNode` trait.

## 2. Module layout

```
src/engine/ui/
├── mod.rs                    # Re-exports (CustomNode, ContentSize, PointerEvent, bridges, registry)
├── custom_node.rs            # CustomNode trait + PointerEvent + ContentSize (component extension point)
└── components/
    ├── mod.rs                # Submodule declarations
    ├── custom_node_bridge.rs # CustomNodeBridge (block/inline unified layout bridge)
    ├── inline_cache.rs       # CSS size resolution for custom nodes
    ├── button.rs             # ButtonComponent
    ├── image.rs              # ImageComponent
    ├── text_input.rs         # InputTextComponent (IME-aware)
    ├── text_input_types.rs   # InputTextEvent / InputTextKey / InputTextState
    └── registry.rs           # ComponentRegistry + CustomNodeFactory
```

### Dependencies between modules

```
                  ┌─────────────────────────────────────┐
                  │           ui_layout crate           │
                  │  CustomLayouter / Style │
                  └───────────────┬─────────────────────┘
                                  │ implements
                                  ▼
┌────────────────────────────────────────────────────────────┐
│ engine::ui                                                  │
│  custom_node.rs ── CustomNode trait                         │
│     ▲            (draw / size / focus / IME extension)      │
│     │ implements                                            │
│  components/                                               │
│   ├── button.rs / image.rs / text_input.rs  ← components    │
│   ├── custom_node_bridge.rs                 ← ui_layout glue │
│   └── inline_cache.rs                       ← sizing│
└────────────────────────────────────────────────────────────┘
          ▲                          ▲
          │ LayoutNode / InfoNode    │ DrawCommand / hit_test
          │ (builder.rs)             │ (renderer_model / input)
          ▼                          ▼
   engine::layouter           engine::renderer_model / input
```

## 3. Core concepts

### 3.1 The `CustomNode` trait (`custom_node.rs`)

Every component implements this trait, which requires `'static` and `Debug`.

| Method                                                             | Role                                                                                                         |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------ |
| `draw_sized(&self, cmd_buf, text_style, style, size: ContentSize)` | **Primary path (required)**: draw fitted to the resolved content-box `size`                                  |
| `draw(&self, cmd_buf, text_style)`                                 | Fallback. Defaults to `draw_sized(cmd, ts, &Style::default(), intrinsic)`                                    |
| `background() -> Option<Background>`                               | Overrides the CSS background when `Some` (solid color or gradient)                                           |
| `intrinsic_size() -> ContentSize`                                  | Natural (content-box) size used when no CSS size is set; CSS resolution lives in `resolve_border_box_size()` |
| `preserves_intrinsic_aspect_ratio() -> bool`                       | Scale the other axis from the intrinsic ratio when one axis is set                                           |
| `accepts_text_input() -> bool`                                     | Whether the node accepts keyboard / IME input                                                                |
| `set_focused(&self, bool)` / `is_focused()`                        | Update / read keyboard focus                                                                                 |
| `handle_text_input(&self, InputTextEvent) -> bool`                 | Handle a platform-neutral editing event                                                                      |
| `is_composing() -> bool`                                           | Whether an IME preedit is active                                                                             |
| `on_pointer_event(&self, PointerEvent) -> bool`                    | Handle a platform-neutral pointer event (`Move` / `Down` / `Up` / `Leave`)                                   |
| `set_hovered(&self, bool)` / `is_hovered()`                        | Update / read the hover state                                                                                |
| `composition_rect() -> Option<(f32, f32, f32, f32)>`               | Content-box rect of the active IME composition underline                                                     |
| `role() -> Option<&'static str>`                                   | Accessibility role (`"button"`, `"textbox"`, `"img"`, …)                                                     |
| `label() -> Option<String>`                                        | Accessibility label (accessible name)                                                                        |
| `value() -> Option<String>`                                        | Current value for editable / stateful nodes                                                                  |
| `is_disabled() -> bool`                                            | Whether the node is disabled and must not receive input                                                      |
| `needs_repaint() -> bool`                                          | Whether the node's visual state changed since the last check (consumes the flag)                             |

#### Coordinate system

Commands emitted by `draw` / `draw_sized` use **content-box coordinates**:
`(0, 0)` is the top-left of the content box. The parent's transform / clip stack
handles positioning.

#### Two-tier drawing

- `draw_sized()` is the **primary path (required)**: the bridge passes the resolved style and size.
- `draw()` is the **fallback** without CSS information. The trait's default
  implementation delegates to
  `self.draw_sized(cmd_buf, text_style, &Style::default(), self.intrinsic_size())`.
  Components drawing only at their intrinsic size may implement `draw()` instead.

### 3.2 Layout bridge (`custom_node_bridge.rs`)

`ui_layout`'s layout engine does not know about `CustomNode`, so a **bridge**
implementing an `ui_layout` trait connects the two sides. A single
`CustomNodeBridge` implements the unified `CustomLayouter` trait; the resolved
`OuterDisplay` read from its owned `ui_layout::Style` selects the formatting
context, and `layout(ctx)` returns a `LayoutBox`:

| `style.display.outer`  | Returned `LayoutBox`                                              |
| ---------------------- | ----------------------------------------------------------------- |
| `OuterDisplay::Block`  | `LayoutBox::BlockBox(BoxModel)` (border-box `Rect` at the origin) |
| `OuterDisplay::Inline` | `LayoutBox::InlineBox(InlineBox)` (spans + box model)             |
| `OuterDisplay::None`   | `LayoutBox::None` (element is skipped)                            |

`measure()` is implemented for every context so flex sizing and auto-height work
regardless of the display value.

The bridge holds `layout_style: ui_layout::Style` (resolved CSS) and passes it to
`resolve_border_box_size()` to compute the final box size. It is exposed via the
`style()` accessor; the layout engine reads `display.outer` from it, so the trait
does not need to report the formatting context.

Custom objects are attached to the layout tree as
`LayoutChild::Custom(Box<CustomChild>)` (the old `LayoutChild::Object` variant no
longer exists); the engine stores each object's `CustomObjectResult` on the
`CustomChild`. Both the renderer and the hit tester read the result back from the
tree via `LayoutChild::custom_result()`, so no side channel (thread-local cache /
`layout_id`) is needed.

### 3.3 Size resolution (`inline_cache.rs`)

`resolve_border_box_size(node, style, containing_width, containing_height, viewport_width, viewport_height) -> ContentSize`
returns the **border-box size** from the resolved CSS `width` / `height` (auto) and
the node's intrinsic size. The content-box resolution is delegated to
`ui_layout::resolve_custom_box_size`, then padding + border are added. It handles:

1. **Intrinsic size** … via `node.intrinsic_size()`
2. **Box-sizing** … `border-box` subtracts padding+border from the CSS size;
   `content-box` adds them
3. **Aspect ratio** … when only one axis is specified, derive the other from the
   CSS `aspect-ratio` (`style.size.aspect_ratio`) falling back to the intrinsic
   ratio if `preserves_intrinsic_aspect_ratio()` is true
4. **Min / max constraints** … apply `min-width` / `max-width` / `min-height` /
   `max-height` (absolute lengths only; percentages are skipped without a
   containing block)

The bridges resolve `vw` / `vh` percentages against
`LayoutContext::viewport_width` / `viewport_height`, which the layout engine fills
from the root layout size.

Inline layout results (`LineSpan` + box model) are carried on the tree as
`CustomObjectResult`, so the renderer and hit tester read them directly from
`LayoutChild::custom_result()`.

### 3.4 `NodeKind::Custom` (`layouter/types.rs`)

Replaced elements appear in the layout tree as `NodeKind::Custom`.

```rust
NodeKind::Custom {
    node: Rc<dyn CustomNode>,        // the actual component
    style: ContainerStyle,           // box painting (background, border, …)
    layout_style: ui_layout::Style,  // resolved CSS layout style (added in Phase 6)
    text_style: TextStyle,           // inherited text style
    scroll_x: bool, scroll_y: bool,
    scroll_offset_x: f32, scroll_offset_y: f32,
}
```

## 4. Build-to-draw flow

```
HTML DOM
   │  builder.rs（CUSTOM_TAGS = ["button", "img", "input"]）
   ▼
Component creation（Rc<dyn CustomNode>）
   │  CustomNodeBridge::new(node, style)（LayoutChild::Custom）
   ▼
LayoutNode + InfoNode（NodeKind::Custom）
   │  ui_layout::LayoutEngine::layout()
   ▼
Block: bridge.layout() → resolve_border_box_size() → LayoutBox::BlockBox(BoxModel)
Inline: bridge.layout()/measure() → LayoutBox::InlineBox (stored on CustomChild)
   │  renderer_model::box_model.rs::generate_draw_commands()
   ▼
Block: node.draw_sized(cmd_buf, text_style, &layout_style, size)
Inline: custom_result() → box model + span → draw_sized(cmd_buf, text_style, &layout_style, (cw, ch))
   ▼
Vec<DrawCommand> → platform::renderer
```

### 4.1 Construction in builder.rs

`build_layout_and_info()` detects `<button>` / `<img>` / `<input>` and creates the
matching component, choosing a bridge based on `display: outer`.

| Tag      | Component            | Initialization                                                                |
| -------- | -------------------- | ----------------------------------------------------------------------------- |
| `button` | `ButtonComponent`    | label (inner_text), background color, text color, text measurer               |
| `img`    | `ImageComponent`     | decodes `Image` from the `src` attribute; `alt` text for fallback             |
| `input`  | `InputTextComponent` | `value` / `placeholder` attributes, text measurer, optional DOM sync callback |

`NodeKind::Custom.layout_style` holds a clone of the `Style`, identical to the one
passed to `LayoutNode::with_children(style, …)`.

### 4.2 Drawing in box_model.rs

`generate_draw_commands()` handles `NodeKind::Custom` in two ways:

- **Block element**: recurses into the child `LayoutNode`, then calls
  `node.draw_sized(cmd_buf, text_style, &layout_style, size)` with the content-box size
  (`layout_style` is `NodeKind::Custom.layout_style`)
- **Inline element**: reads the `CustomObjectResult` from the tree child
  (`LayoutChild::Custom(CustomChild) → custom_result()`), uses its box model and
  `LineSpan`, and calls
  `node.draw_sized(cmd_buf, text_style, &layout_style, ContentSize { width: cw, height: ch })`

`push_box_model()` / `pop_box_model()` manage transform / clip push-pops uniformly for
all boxes, so `CustomNode` only draws in content-box coordinates.

## 5. Input flow (`engine::input`)

`engine::input::hit_test()` walks `LayoutNode` + `InfoNode` to find a hit path, then
`focus_text_input()` / `dispatch_text_input()` invoke `CustomNode`'s
`accepts_text_input()` / `set_focused()` / `handle_text_input()`.

```
key / IME events
   ▼ engine::input
   ▼ hit_test → finds NodeKind::Custom
   ▼ focus_text_input → set_focused()
   ▼ dispatch_text_input → handle_text_input(InputTextEvent)
   ▼ InputTextComponent（updates value / caret / preedit）
```

`InputTextEvent` is a platform-neutral editing event passed from the `platform`
input device implementation into `engine` (`Insert` / `Preedit` / `Commit` /
`Key` / `Enter` / `Undo` / `Redo` / `Paste` / `CancelComposition`).

Pointer events (`PointerEvent`: `Move` / `Down` / `Up` / `Leave`) are dispatched
to the innermost `Custom` node on the hit path via `engine::input::dispatch_pointer()`.
`engine::input::update_hover()` manages the hover transition when the pointer moves.

`engine::input::any_custom_node_needs_repaint()` walks the tree to check whether
any custom node has a pending visual change (dirty flag). This is used to suppress
unnecessary full redraws when nothing changed.

## 6. Component reference

### 6.1 `ButtonComponent` (`button.rs`)

```rust
pub struct ButtonComponent {
    pub label: String,
    pub button_color: Color,
    pub label_color: Color,
    // private: measurer, hovered / pressed / dirty state (Cell<bool>)
}
```

- Drawing: background via `background()` (`button_color`, shaded on hover/press);
  label via `DrawText`
- Size: `intrinsic_size()` is measured from the label text using the `TextMeasurer`;
  CSS width/height apply through `resolve_border_box_size()`
- The label is vertically centered inside `draw_sized()`
- Input: `on_pointer_event()` tracks hover / pressed; `Up` after `Down` reports a click
- A11y: `role() = "button"`, `label()` = label text
- Dirty tracking: `needs_repaint()` returns whether the visual state changed since last check

### 6.2 `InputTextComponent` (`text_input.rs`)

```rust
pub struct InputTextComponent {
    state: RefCell<InputTextState>,      // value / caret / preedit / focused
    placeholder: SmolStr,
    measurer: Arc<dyn TextMeasurer>,
    // private: undo / redo history, dirty flag, on_value_change callback
}
```

- Size: `intrinsic_size()` = `ContentSize { width: 200.0, height: 28.0 }`
- Drawing: emits text, caret, and IME preedit underline (`draw_text_input()`
  measures glyph widths with the measurer for layout)
- Input: `accepts_text_input() = true`; handles Backspace / Delete / arrows / Home / End
- IME: `Preedit` / `Commit` / `CancelComposition` managed via `InputTextState.preedit`;
  `composition_rect()` reports the preedit underline rect in content-box coordinates
- Editing: `Enter` clears preedit; `Undo` / `Redo` walk the edit history; `Paste` inserts
  at the caret
- DOM sync: `on_value_change` callback (wired by the factory) updates the DOM `value`
  attribute when the user edits the input
- A11y: `role() = "textbox"`, `label()` = placeholder, `value()` = current value

### 6.3 `ImageComponent` (`image.rs`)

```rust
pub struct ImageComponent {
    pub image: Option<Image>,
    pub alt: String,
}
```

- Drawing: fills `rect_path(0, 0, size.width, size.height)` with `Brush::Image`;
  when the image failed to decode, renders a placeholder box with the `alt` text
- Size: `intrinsic_size()` is the image's pixel dimensions;
  `preserves_intrinsic_aspect_ratio() = true` keeps the intrinsic ratio when one axis is set
  (min/max constraints are applied by `resolve_border_box_size()`); for broken images,
  the intrinsic size is based on the alt text
- A11y: `role() = "img"` when the image is present; `label()` = alt text

## 7. Adding a new component

1. Add a struct implementing `Rc<dyn CustomNode>` under `components/`
   (implement `engine::ui::custom_node::CustomNode`)
2. Add `pub mod` in `components/mod.rs`
3. Register a [`CustomNodeFactory`] in `registry.rs` — this drives both the tag list
   (`ComponentRegistry::tags()`, used by `builder.rs` instead of `CUSTOM_TAGS`) and
   construction. No `builder.rs` `match tag` changes needed.
4. Reuse the existing block / inline bridges — they hold a `CustomNode`, so they
   usually work as-is
5. `generate_draw_commands()` already handles `NodeKind::Custom`; no changes needed

## 8. Design constraints

- **Content-box coordinates**: components draw only their own content; positioning is
  delegated to the parent's transform stack
- **`draw` vs `draw_sized`**: `draw` is the fallback, `draw_sized` is the primary path.
  CSS size reaches the component via bridge → `resolve_border_box_size` → `draw_sized(size)`
- **`engine` must not reference `platform`**: input is abstracted as platform-neutral
  `InputTextEvent` / `PointerEvent`; text measurement goes through the
  `bridge::text::TextMeasurer` trait
- **`ui_layout` dependency**: the bridges and `Style` depend on the `ui_layout` crate,
  pinned to a git revision. Do not bump casually.
- **`ContentSize` type**: sizes are expressed as `ContentSize { width, height }` to
  distinguish content-box dimensions at the type level; tuple `(f32, f32)` is still used
  internally in some helpers but will migrate over time
