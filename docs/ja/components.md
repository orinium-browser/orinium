# コンポーネントアーキテクチャ（`engine::ui`）

## 1. 概要

`engine::ui` は主に HTML の**置換要素 (replaced element)** — `<button>` / `<img>` / `<input>` など —
の描画・レイアウト・入力処理を担当する UI コンポーネント層です。

通常の要素は `engine::layouter` が DOM+CSS から `InfoNode` / `LayoutNode` を構築し、
`engine::renderer_model` が `DrawCommand` を生成します。一方、置換要素は
外部の実装（ネイティブ UI・画像デコーダ・IME 対応ウィジェット）に描画を委譲するため、
`CustomNode` トレイトという拡張ポイントを介してレイアウトエンジンと接続します。

## 2. モジュール構成

```
src/engine/ui/
├── mod.rs                    # 再エクスポート（CustomNode, ContentSize, PointerEvent, ブリッジ, レジストリ）
├── custom_node.rs            # CustomNode トレイト + PointerEvent + ContentSize（コンポーネントの拡張点）
└── components/
    ├── mod.rs                # サブモジュール宣言
    ├── custom_node_bridge.rs # CustomNodeBridge（ブロック/インライン統合レイアウトブリッジ）
    ├── inline_cache.rs       # カスタムノードの CSS サイズ解決
    ├── button.rs             # ButtonComponent
    ├── image.rs              # ImageComponent
    ├── text_input.rs         # InputTextComponent（IME 対応）
    ├── text_input_types.rs   # InputTextEvent / InputTextKey / InputTextState
    └── registry.rs           # ComponentRegistry + CustomNodeFactory
```

### モジュール間の依存関係

```
                  ┌─────────────────────────────────────┐
                  │           ui_layout crate           │
                  │  CustomLayouter / Style │
                  └───────────────┬─────────────────────┘
                                  │ 実装
                                  ▼
┌────────────────────────────────────────────────────────────┐
│ engine::ui                                                 │
│  custom_node.rs ── CustomNode トレイト                       │
│     ▲            （描画・サイズ・フォーカス・IME の拡張点）       │
│     │ 実装                                                   │
│  components/                                                │
│   ├── button.rs / image.rs / text_input.rs ← コンポーネント   │
│   ├── custom_node_bridge.rs                ← ui_layout 接続 │
│   └── inline_cache.rs                      ← サイズ解決│
└────────────────────────────────────────────────────────────┘
          ▲                          ▲
          │ LayoutNode / InfoNode    │ DrawCommand / hit_test
          │ (builder.rs)             │ (renderer_model / input)
          ▼                          ▼
   engine::layouter           engine::renderer_model / input
```

## 3. コア概念

### 3.1 `CustomNode` トレイト（`custom_node.rs`）

すべてのコンポーネントが実装するトレイトです。`'static` と `Debug` を要求します。

| メソッド                                              | 役割                                                                                             |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `draw_sized(&self, cmd_buf, text_style, style, size)` | **主経路（必須）**。解決済み content-box サイズ `size` に合わせて描画                            |
| `draw(&self, cmd_buf, text_style)`                    | フォールバック。デフォルトで `draw_sized(cmd, ts, &Style::default(), intrinsic)` に委譲          |
| `background() -> Option<Background>`                  | 返した背景（単色 or グラデーション）で CSS の背景を上書き                                        |
| `intrinsic_size() -> ContentSize`                     | CSS でサイズ未指定時の自然サイズ（content-box）。CSS 解釈は `resolve_border_box_size()` に一本化 |
| `preserves_intrinsic_aspect_ratio() -> bool`          | 片側指定時にもう片側を固有アスペクト比に従わせる                                                 |
| `accepts_text_input() -> bool`                        | キーボード / IME 入力を受け付けるか                                                              |
| `set_focused(&self, bool)` / `is_focused()`           | フォーカス状態の更新・参照                                                                       |
| `handle_text_input(&self, InputTextEvent) -> bool`    | プラットフォーム非依存の編集イベントを処理                                                       |
| `is_composing() -> bool`                              | IME プレエディット（変換中）かどうか                                                             |
| `on_pointer_event(&self, PointerEvent) -> bool`       | プラットフォーム非依存のポインタイベント（`Move`/`Down`/`Up`/`Leave`）を処理                     |
| `set_hovered(&self, bool)` / `is_hovered()`           | ホバー状態の更新・参照                                                                           |
| `composition_rect() -> Option<(f32, f32, f32, f32)>`  | 変換中下線の content-box 座標矩形                                                                |
| `role() -> Option<&'static str>`                      | アクセシビリティの役割（`"button"` / `"textbox"` / `"img"` など）                                |
| `label() -> Option<String>`                           | アクセシビリティのラベル（アクセシブル名）                                                       |
| `value() -> Option<String>`                           | 編集可能 / 状態を持つノードの現在値                                                              |
| `is_disabled() -> bool`                               | 無効化されており入力を受け付けないか                                                             |
| `needs_repaint() -> bool`                             | 直前のチェック以降に視覚状態が変化したか（フラグを消費する）                                     |

#### 座標系

`draw` / `draw_sized` が生成するコマンドは **content-box 座標系**で記述します。
`(0, 0)` は content-box の左上。位置合わせは親の transform / clip スタックが担当します。

#### 描画とサイズの2段構え

- `draw_sized()` はブリッジ経由で実スタイル・実サイズが渡される**主経路（必須メソッド）**
- `draw()` は CSS 情報を持たない**フォールバック**で、トレイトのデフォルト実装が
  `self.draw_sized(cmd_buf, text_style, &Style::default(), self.intrinsic_size())`
  に委譲します。intrinsic サイズだけで描画するコンポーネントは `draw()` 側を実装してもよい

### 3.2 レイアウトブリッジ（`custom_node_bridge.rs`）

`CustomNode` は `ui_layout` のレイアウトエンジンに直接は認識されないため、
`ui_layout` が提供するトレイトを実装した**ブリッジ**が両者をつなぎます。
単一の `CustomNodeBridge` が統一トレイト `CustomLayouter` を実装し、構築時に
保持する解決済み `ui_layout::Style` の `display.outer` が整形文脈を選び、
`layout(ctx)` で `LayoutBox` を返します:

| `style.display.outer` | 返す `LayoutBox`                                            |
| ---------------------- | ----------------------------------------------------------- |
| `OuterDisplay::Block`  | `LayoutBox::BlockBox(BoxModel)`（原点の border-box `Rect`） |
| `OuterDisplay::Inline` | `LayoutBox::InlineBox(InlineBox)`（spans + box model）      |
| `OuterDisplay::None`   | `LayoutBox::None`（要素はスキップ）                         |

`measure()` は全コンテキストで実装しており、display 値によらず flex の
sizing と auto-height が機能します。

ブリッジは `layout_style: ui_layout::Style`（CSS 解決済みスタイル）を持ち、
`resolve_border_box_size()` に渡して実際のボックスサイズを求めます。
`style()` アクセサで外部から参照できます（レイアウトエンジンはこれを読んで
`display.outer` を判定するため、トレイトに整形文脈を問い合わせる必要はありません）。

カスタムオブジェクトは `LayoutChild::Custom(Box<CustomChild>)` として
レイアウトツリーに組み込まれます（旧 `LayoutChild::Object` は廃止）。エンジンは
各オブジェクトの `CustomObjectResult` を `CustomChild` に保存し、描画層・ヒットテストは
`LayoutChild::custom_result()` でツリーから直接読み取るため、
スレッドローカルキャッシュや `layout_id` は不要です。

### 3.3 サイズ解決（`inline_cache.rs`）

`resolve_border_box_size(node, style, containing_width, containing_height, viewport_width, viewport_height) -> ContentSize`
は、CSS の `width` / `height`（auto）と `CustomNode` の intrinsic サイズから
**border-box サイズ**を返します。content-box の解決は ui_layout の
`ui_layout::resolve_custom_box_size` に委譲し、padding + border を加算します:

1. **intrinsic サイズ** … `node.intrinsic_size()` を利用
2. **box-sizing** … `border-box` なら CSS サイズから padding+border を引き、
   `content-box` なら padding+border を加える
3. **アスペクト比** … 片側だけ指定された場合、CSS の `aspect-ratio`
   （`style.size.aspect_ratio`）を優先し、未指定なら `preserves_intrinsic_aspect_ratio()`
   が真のとき固有比でもう片方を決定
4. **min / max 制約** … `min-width` / `max-width` / `min-height` / `max-height` を適用
   （絶対長のみ解決。% は包含ブロックが無いためスキップ）

ブリッジは `vw` / `vh` を `LayoutContext::viewport_width` / `viewport_height` に対して
解決します（ルートのレイアウトサイズからレイアウトエンジンが設定）。

インライン要素のレイアウト結果（`LineSpan` と box model）はツリー上で
`CustomObjectResult` として保持され、描画層・ヒットテストは
`LayoutChild::custom_result()` から直接参照します。

### 3.4 `NodeKind::Custom`（`layouter/types.rs`）

置換要素はレイアウトツリー上で `NodeKind::Custom` として表現されます。

```rust
NodeKind::Custom {
    node: Rc<dyn CustomNode>,        // 実際のコンポーネント
    style: ContainerStyle,           // ボックス描画用（背景・ボーダー等）
    layout_style: ui_layout::Style,  // CSS 解決済みレイアウトスタイル（Phase 6 で追加）
    text_style: TextStyle,           // 継承されたテキストスタイル
    scroll_x: bool, scroll_y: bool,  // スクロールフラグ
    scroll_offset_x: f32, scroll_offset_y: f32,
}
```

## 4. ビルドから描画までのフロー

```
HTML DOM
   │  builder.rs（CUSTOM_TAGS = ["button", "img", "input"]）
   ▼
コンポーネント生成（Rc<dyn CustomNode>）
   │  CustomNodeBridge::new(node, style)（LayoutChild::Custom）
   ▼
LayoutNode + InfoNode（NodeKind::Custom）
   │  ui_layout::LayoutEngine::layout()
   ▼
ブロック: ブリッジの layout() → resolve_border_box_size() → LayoutBox::BlockBox(BoxModel)
インライン: ブリッジの layout()/measure() → LayoutBox::InlineBox（CustomChild に保持）
   │  renderer_model::box_model.rs::generate_draw_commands()
   ▼
ブロック: node.draw_sized(cmd_buf, text_style, &layout_style, size)
インライン: custom_result() → box model + span → draw_sized(cmd_buf, text_style, &layout_style, (cw, ch))
   ▼
Vec<DrawCommand> → platform::renderer
```

### 4.1 builder.rs での生成

`build_layout_and_info()` は `<button>` / `<img>` / `<input>` を検出すると
対応するコンポーネントを生成し、`display: outer` に応じてブリッジを選びます。

| タグ     | コンポーネント       | 主な初期化                                       |
| -------- | -------------------- | ------------------------------------------------ |
| `button` | `ButtonComponent`    | ラベル（inner_text）、背景色、テキスト色         |
| `img`    | `ImageComponent`     | `src` 属性からデコード済み `Image` を引き当て    |
| `input`  | `InputTextComponent` | `value` / `placeholder` 属性、テキストメジャーラ |

`NodeKind::Custom.layout_style` には `Style` の clone が入り、
`LayoutNode::with_children(style, …)` の `style` と同一の値が保存されます。

### 4.2 box_model.rs での描画

`generate_draw_commands()` は `NodeKind::Custom` を次の 2 通りに処理します。

- **ブロック要素**: 子 `LayoutNode` へ再帰し、content-box サイズを求めて
  `node.draw_sized(cmd_buf, text_style, &layout_style, size)` を呼ぶ
  （`layout_style` は `NodeKind::Custom.layout_style` を参照）
- **インライン要素**: ツリー子要素の `CustomObjectResult`（`LayoutChild::Custom(CustomChild)` → `custom_result()`）を読み、box model と `LineSpan` から
  `node.draw_sized(cmd_buf, text_style, &layout_style, ContentSize { width: cw, height: ch })` を呼ぶ

`push_box_model()` / `pop_box_model()` による transform / clip の push/pop は
すべてのボックスで共通で、`CustomNode` は content-box 座標だけで描画します。

## 5. 入力フロー（`engine::input`）

`engine::input::hit_test()` は `LayoutNode` と `InfoNode` からヒットパスを求め、
`focus_text_input()` / `dispatch_text_input()` が `CustomNode` の
`accepts_text_input()` / `set_focused()` / `handle_text_input()` を呼びます。

```
キー/IME イベント
   ▼ engine::input
   ▼ hit_test → NodeKind::Custom を検出
   ▼ focus_text_input → set_focused()
   ▼ dispatch_text_input → handle_text_input(InputTextEvent)
   ▼ InputTextComponent（状態更新: value / caret / preedit）
```

`InputTextEvent` はプラットフォーム非依存の編集イベントで、
`platform` 側の入力デバイス実装から `engine` へ渡されます
（`Insert` / `Preedit` / `Commit` / `Key` / `Enter` / `Undo` / `Redo` / `Paste` / `CancelComposition`）。

ポインタイベント（`PointerEvent`: `Move` / `Down` / `Up` / `Leave`）は
ヒットパス上の最内 `Custom` ノードへ `engine::input::dispatch_pointer()` で配送されます。
`engine::input::update_hover()` がポインタ移動時のホバー遷移を管理します。

## 6. コンポーネント一覧

### 6.1 `ButtonComponent`（`button.rs`）

```rust
pub struct ButtonComponent {
    pub label: String,
    pub button_color: Color,
    pub label_color: Color,
    // private: measurer, hovered / pressed / dirty 状態（Cell<bool>）
}
```

- 描画: 背景は `background()`（`button_color`。hover / press で明暗をシフト）、ラベルは `DrawText`
- サイズ: `intrinsic_size()` はラベルを `TextMeasurer` で実測。CSS の width/height は
  `resolve_border_box_size()` 経由で適用
- ラベルは `draw_sized()` 内で縦方向に中央揃え
- 入力: `on_pointer_event()` で hover / pressed を追跡。`Down` 後の `Up` がクリックを報告
- アクセシビリティ: `role() = "button"`、`label()` = ラベル文字列
- dirty 追跡: `needs_repaint()` は直前のチェック以降に視覚状態が変化したかを返す（フラグを消費）

### 6.2 `InputTextComponent`（`text_input.rs`）

```rust
pub struct InputTextComponent {
    state: RefCell<InputTextState>,      // value / caret / preedit / focused
    placeholder: SmolStr,
    measurer: Arc<dyn TextMeasurer<TextStyle>>,
    // private: undo / redo 履歴、dirty フラグ、on_value_change コールバック
}
```

- サイズ: `intrinsic_size()` = `ContentSize { width: 200.0, height: 28.0 }`
- 描画: テキスト、キャレット、IME プレエディット下線を生成
  （`draw_text_input()` がメジャーラで字形幅を測定して配置）
- 入力: `accepts_text_input() = true`。バックスペース / Delete / 矢印 / Home / End を処理
- IME: `Preedit` / `Commit` / `CancelComposition` を `InputTextState.preedit` で管理。
  `composition_rect()` が変換中下線の content-box 座標矩形を返す
- 編集: `Enter` は preedit をクリア。`Undo` / `Redo` は編集履歴を辿る。`Paste` はキャレット位置へ挿入
- DOM 同期: `on_value_change` コールバック（ファクトリで配線）がユーザー編集時に DOM `value` 属性を更新
- アクセシビリティ: `role() = "textbox"`、`label()` = placeholder、`value()` = 現在値

### 6.3 `ImageComponent`（`image.rs`）

```rust
pub struct ImageComponent {
    pub image: Option<Image>,
    pub alt: String,
}
```

- 描画: `Brush::Image` で `rect_path(0, 0, size.width, size.height)` を塗る。
  画像がデコードに失敗した場合（`image == None`）は `alt` テキスト付きのプレースホルダーボックスを描画
- サイズ: `intrinsic_size()` は画像のピクセル寸法。
  `preserves_intrinsic_aspect_ratio() = true` で、片側指定時は固有比を維持
  （min/max 制約は `resolve_border_box_size()` が適用）。破損画像の場合は alt テキストに基づく固定サイズ
- アクセシビリティ: `role() = "img"`（画像あり時）、`label()` = alt テキスト

## 7. 新コンポーネントの追加方法

1. `components/` に `Rc<dyn CustomNode>` を実装した構造体を追加する
   （`engine::ui::custom_node::CustomNode` を実装）
2. `components/mod.rs` に `pub mod` を追加
3. `registry.rs` に [`CustomNodeFactory`] を登録する。これがタグ一覧
   （`ComponentRegistry::tags()`、`builder.rs` は `CUSTOM_TAGS` の代わりに使用）と
   生成の両方を駆動するため、`builder.rs` の `match tag` は変更不要
4. 必要に応じてブロック / インライン用のブリッジを再利用する
   （ブリッジは `CustomNode` を保持するため通常はそのままで動く）
5. `generate_draw_commands()` は `NodeKind::Custom` を既に処理しているため変更不要

## 8. 設計上の注意点

- **content-box 座標系**: コンポーネントは自身のコンテンツだけを描き、位置は親の transform に任せる
- **`draw` vs `draw_sized`**: `draw` はフォールバック、`draw_sized` が主経路。
  CSS サイズはブリッジ → `resolve_border_box_size` → `draw_sized(size)` で届く
- **`engine` は `platform` を参照しない**: 入力イベントはプラットフォーム非依存の
  `InputTextEvent` / `PointerEvent` で抽象化し、計測は `bridge::text::TextMeasurer` トレイト経由
- **`ui_layout` クレート依存**: ブリッジと `Style` は git 依存の `ui_layout` に固定（rev 指定）。
  不用意に bump しない
- **`ContentSize` 型**: サイズは `ContentSize { width, height }` で型レベルで content-box を明示。
  将来的に内部の tuple `(f32, f32)` も置き換え予定
