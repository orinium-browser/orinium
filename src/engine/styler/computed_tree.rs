use crate::engine::css::values::{Border, Color, Display, Length};
use crate::html::HtmlNodeType;
use std::cell::RefCell;
use std::rc::Weak;

use crate::engine::bridge::text;
use crate::engine::renderer::render_node::RenderTree;
use crate::engine::tree::{Tree, TreeNode};

pub type ComputedTree = Tree<ComputedStyleNode>;

/// 計算済みスタイルを持つノード
#[derive(Debug, Clone)]
pub struct ComputedStyleNode {
    pub html: Weak<RefCell<TreeNode<HtmlNodeType>>>,
    pub computed: Option<ComputedStyle>,
}

impl ComputedTree {
    /// フォールバックの測定器でレイアウトを行い RenderTree を返す
    pub fn layout_with_fallback(&self, root_width: f32, root_height: f32) -> RenderTree {
        let fallback = text::EngineFallbackTextMeasurer::default();
        RenderTree::from_computed_tree_with_measurer(
            self,
            &fallback,
            root_width,
            root_height,
        )
    }

    /// 指定の TextMeasurer でレイアウトを行い RenderTree を返す
    pub fn layout_with_measurer(
        &self,
        measurer: &dyn text::TextMeasurer,
        root_width: f32,
        root_height: f32,
    ) -> RenderTree {
        RenderTree::from_computed_tree_with_measurer(
            self,
            measurer,
            root_width,
            root_height,
        )
    }

    // レイアウト/構造変換はレンダーツリーの責務へ委譲
}

/// 計算済みスタイル
#[derive(Debug, Clone, Default)]
pub struct ComputedStyle {
    pub display: Display,
    pub width: Option<Length>,
    pub height: Option<Length>,

    pub margin_top: Length,
    pub margin_right: Length,
    pub margin_bottom: Length,
    pub margin_left: Length,

    pub padding_top: Length,
    pub padding_right: Length,
    pub padding_bottom: Length,
    pub padding_left: Length,

    pub color: Option<Color>,
    pub background_color: Option<Color>,

    pub border: Option<Border>,

    pub font_size: Option<Length>,
}

impl ComputedStyle {
    /// 指定された長さをピクセルで解決する
    /// - `available` はパーセンテージ解決時の基準（幅/高さに対する親の利用可能値）
    /// - `base_font` は `em` 等の相対単位解決に用いる基準（px）
    pub fn resolve_length_px_option(
        length: Option<Length>,
        available: f32,
        base_font: f32,
    ) -> Option<f32> {
        match length {
            Some(l) => match l {
                Length::Percent(_) => l.to_px_option(available),
                Length::Em(_) => l.to_px_option(base_font),
                _ => l.to_px_option(base_font),
            },
            None => None,
        }
    }

    /// 計算済みスタイルの幅をピクセルで解決する（指定がなければ None を返す）
    pub fn resolved_width_px(&self, available_width: f32, base_font: f32) -> Option<f32> {
        Self::resolve_length_px_option(self.width, available_width, base_font)
    }

    /// 計算済みスタイルの高さをピクセルで解決する（指定がなければ None を返す）
    pub fn resolved_height_px(&self, available_height: f32, base_font: f32) -> Option<f32> {
        Self::resolve_length_px_option(self.height, available_height, base_font)
    }
}
