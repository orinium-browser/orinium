use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ui_layout::{
    BoxModel, CustomLayouter, InlineBox, LayoutBox, LayoutContext, LineSpan, MeasureResult, Rect,
};

use crate::engine::bridge::text::GlyphCluster;
use crate::engine::layouter::types::TextStyle;

thread_local! {
    static TEXT_RESULTS: RefCell<HashMap<usize, Arc<TextLayoutResult>>> =
        RefCell::new(HashMap::new());
}

static NEXT_TEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// Result of laying out a text chunk into lines.
#[derive(Debug, Clone)]
pub struct TextLayoutResult {
    /// Per-line spans (positions and extents).
    pub spans: Vec<LineSpan>,
    /// Per-line text strings (one per span).
    pub line_texts: Vec<String>,
}

/// A self-layouting text object that implements [`CustomLayouter`].
///
/// Constructed with pre-shaped cluster data from a text measurer.
/// During layout it wraps text at word boundaries using
/// `available_inline_size`. Results are cached in a thread-local store
/// keyed by a unique ID for the rendering layer to consume.
#[derive(Debug)]
pub struct TextFlowLayouter {
    /// Unique identifier for cache lookup.
    pub id: usize,
    text: String,
    clusters: Vec<GlyphCluster>,
    line_height: f32,
}

impl TextFlowLayouter {
    pub fn new(
        text: String,
        _style: TextStyle,
        mut clusters: Vec<GlyphCluster>,
        line_height: f32,
    ) -> Self {
        clusters.sort_by_key(|c| c.byte_offset);
        let id = NEXT_TEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            id,
            text,
            clusters,
            line_height,
        }
    }

    /// Retrieve the layout result for `id` from the thread-local cache.
    pub fn get_result(id: usize) -> Option<Arc<TextLayoutResult>> {
        TEXT_RESULTS.with(|cache| cache.borrow().get(&id).cloned())
    }

    fn remove_result(id: usize) {
        TEXT_RESULTS.with(|cache| {
            cache.borrow_mut().remove(&id);
        });
    }

    fn compute_layout(
        &self,
        available_first_line_space: f32,
        available_space: f32,
        start_pos: (f32, f32),
    ) -> TextLayoutResult {
        let lh = self.line_height.max(1.0);
        let text_len = self.text.len();
        let clusters = &self.clusters;

        // Edge-case: no clusters but non-empty text (e.g. spaces-only).
        if clusters.is_empty() && !self.text.is_empty() {
            return TextLayoutResult {
                spans: vec![LineSpan {
                    x_range: start_pos.0..start_pos.0,
                    line_pos: start_pos,
                    line_index: 0,
                }],
                line_texts: vec![self.text.clone()],
            };
        }
        if clusters.is_empty() {
            return TextLayoutResult {
                spans: Vec::new(),
                line_texts: Vec::new(),
            };
        }

        // Resolved line widths. The first line may share its line with
        // preceding inline content, so it can be narrower than subsequent
        // lines. If a width is unknown (zero at the start of a layout pass
        // with no prior content), fall back to the other one, or to a large
        // value so nothing wraps unexpectedly.
        let first_line_width = if available_first_line_space > 0.0 {
            available_first_line_space
        } else {
            available_space
        };
        let line_width = |line_index: usize| {
            if line_index == 0 {
                first_line_width
            } else {
                available_space
            }
        };

        let mut spans: Vec<LineSpan> = Vec::new();
        let mut line_texts: Vec<String> = Vec::new();

        let mut line_start: usize = 0; // byte offset where current line starts
        let mut line_start_idx: usize = 0; // cluster index where current line starts
        let mut x_pos = start_pos.0;
        let mut y_pos = start_pos.1;
        let mut line_index: usize = 0;
        let mut accumulated: f32 = 0.0;
        let mut last_breakable_cluster: Option<usize> = None; // cluster index (exclusive)

        macro_rules! emit_line {
            ($end_idx:expr, $end_byte:expr) => {{
                let end_idx = $end_idx;
                let end_byte = $end_byte;

                let line_w: f32 = if end_idx > line_start_idx {
                    clusters[line_start_idx..end_idx]
                        .iter()
                        .map(|c| c.width)
                        .sum()
                } else {
                    0.0
                };

                let line_str = &self.text[line_start..end_byte];
                let trimmed = line_str.trim_end_matches('\n');
                let line_text = if trimmed.is_empty() {
                    String::new()
                } else {
                    trimmed.to_string()
                };

                spans.push(LineSpan {
                    x_range: x_pos..(x_pos + line_w),
                    line_pos: (x_pos, y_pos),
                    line_index,
                });
                line_texts.push(line_text);
            }};
        }

        let mut i = 0;
        while i < clusters.len() {
            let frag = &clusters[i];
            let next_byte = clusters
                .get(i + 1)
                .map(|f| f.byte_offset)
                .unwrap_or(text_len);

            // Check for a forced newline in the text spanned by this cluster
            let cluster_text = &self.text[frag.byte_offset..next_byte];
            let nl_pos = cluster_text.find('\n');

            if let Some(nl_offset) = nl_pos {
                // The cluster itself contains a newline.
                // If we have content before the newline on this line, emit it.
                let nl_byte = frag.byte_offset + nl_offset;

                if nl_byte > line_start {
                    emit_line!(i, nl_byte);

                    // Advance to next line state for content after \n
                    y_pos += lh;
                    line_index += 1;
                    x_pos = 0.0;

                    let next_byte_after_nl = nl_byte + 1;
                    if next_byte_after_nl < next_byte {
                        line_start = next_byte_after_nl;
                        line_start_idx = i;
                        accumulated = 0.0;
                        last_breakable_cluster = None;
                    } else {
                        line_start = nl_byte + 1;
                        line_start_idx = i + 1;
                        accumulated = 0.0;
                        last_breakable_cluster = None;
                    }
                } else {
                    // Empty line (e.g. consecutive newlines)
                    spans.push(LineSpan {
                        x_range: x_pos..x_pos,
                        line_pos: (x_pos, y_pos),
                        line_index,
                    });
                    line_texts.push(String::new());
                    line_start = nl_byte + 1;
                    line_start_idx = i + 1;
                    x_pos = 0.0;
                    y_pos += lh;
                    line_index += 1;
                    accumulated = 0.0;
                    last_breakable_cluster = None;
                }

                i += 1;
                continue;
            }

            // Check if placing this cluster would overflow the line.
            // Break at the last known breakable cluster, or at the current
            // cluster if nothing earlier was breakable (unbreakable run).
            let current_line_width = line_width(line_index);
            if accumulated > 0.0 && accumulated + frag.width > current_line_width {
                let break_at = last_breakable_cluster.unwrap_or(i);

                let break_byte = if break_at < clusters.len() {
                    clusters[break_at].byte_offset
                } else {
                    text_len
                };

                if break_byte > line_start || spans.is_empty() {
                    emit_line!(break_at, break_byte);

                    line_start = break_byte;
                    line_start_idx = break_at;
                    x_pos = 0.0;
                    y_pos += lh;
                    line_index += 1;
                    last_breakable_cluster = None;

                    // Carry over the width of any non-breakable clusters that
                    // move to the new line together with this one. break_at <=
                    // i, so the range is empty in the common cases.
                    accumulated = if break_at < i {
                        clusters[break_at..i].iter().map(|c| c.width).sum()
                    } else {
                        0.0
                    };
                }
            }

            // Accumulate width
            accumulated += frag.width;

            if frag.break_allowed {
                last_breakable_cluster = Some(i + 1);
            }

            i += 1;
        }

        // Emit any remaining text as the last line
        if line_start < text_len {
            let line_w: f32 = if line_start_idx < clusters.len() {
                clusters[line_start_idx..].iter().map(|c| c.width).sum()
            } else {
                0.0
            };
            let line_str = &self.text[line_start..];
            let trimmed = line_str.trim_end_matches('\n');
            spans.push(LineSpan {
                x_range: x_pos..(x_pos + line_w),
                line_pos: (x_pos, y_pos),
                line_index,
            });
            line_texts.push(trimmed.to_string());
        } else if spans.is_empty() {
            // Empty text: produce one empty line so the node contributes height
            spans.push(LineSpan {
                x_range: start_pos.0..start_pos.0,
                line_pos: start_pos,
                line_index: 0,
            });
            line_texts.push(String::new());
        }

        TextLayoutResult { spans, line_texts }
    }
}

impl Drop for TextFlowLayouter {
    fn drop(&mut self) {
        Self::remove_result(self.id);
    }
}

impl CustomLayouter for TextFlowLayouter {
    fn layout(&mut self, ctx: &LayoutContext) -> LayoutBox {
        let result = self.compute_layout(
            ctx.available_inline_size,
            ctx.containing_block_width.unwrap_or(f32::MAX),
            ctx.start_pos,
        );
        let spans = result.spans.clone();

        TEXT_RESULTS.with(|cache| {
            cache.borrow_mut().insert(self.id, Arc::new(result));
        });

        let (start_x, start_y) = ctx.start_pos;
        let lh = self.line_height.max(1.0);
        let total_width = spans
            .iter()
            .map(|s| s.line_pos.0 + s.width())
            .filter(|x| !x.is_nan())
            .max_by(f32::total_cmp)
            .map(|max_x| (max_x - start_x).max(0.0))
            .unwrap_or(0.0);
        let total_height = spans
            .iter()
            .map(|s| s.line_index)
            .max()
            .map(|line| (line as f32 + 1.0) * lh)
            .unwrap_or(0.0);
        let rect = Rect {
            x: start_x,
            y: start_y,
            width: total_width,
            height: total_height,
        };
        let box_model = BoxModel {
            sticky_edges: None,
            border_box: rect,
            padding_box: rect,
            content_box: rect,
            children_box: rect,
        };

        LayoutBox::InlineBox(InlineBox {
            box_model,
            line_spans: spans,
        })
    }

    fn measure(&self, _ctx: &LayoutContext) -> MeasureResult {
        let total_width: f32 = self.clusters.iter().map(|c| c.width).sum();
        let total_height = self.line_height.max(1.0);
        MeasureResult {
            width: total_width,
            height: total_height,
        }
    }

    fn write_debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextFlowLayouter [{}]", self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster(byte_offset: usize, width: f32, break_allowed: bool) -> GlyphCluster {
        GlyphCluster {
            byte_offset,
            width,
            break_allowed,
        }
    }

    fn layout(text: &str, clusters: Vec<GlyphCluster>, line_width: f32) -> TextLayoutResult {
        TextFlowLayouter::new(text.to_string(), TextStyle::default(), clusters, 16.0)
            .compute_layout(line_width, line_width, (0.0, 0.0))
    }

    #[test]
    fn wraps_at_word_boundaries() {
        let result = layout(
            "aaa bbb ccc",
            vec![
                cluster(0, 30.0, false),
                cluster(3, 5.0, true),
                cluster(4, 30.0, false),
                cluster(7, 5.0, true),
                cluster(8, 30.0, false),
            ],
            70.0,
        );
        assert_eq!(result.line_texts.len(), 2);
        assert_eq!(result.line_texts[0].trim_end(), "aaa bbb");
        assert_eq!(result.line_texts[1], "ccc");
        assert_eq!(result.spans[0].width(), 70.0);
        assert_eq!(result.spans[1].width(), 30.0);
        assert_eq!(result.spans[0].line_index, 0);
        assert_eq!(result.spans[1].line_index, 1);
    }

    #[test]
    fn wraps_cjk_per_character() {
        let clusters: Vec<GlyphCluster> = (0..10).map(|i| cluster(i, 10.0, true)).collect();
        let result = layout("aaaaaaaaaa", clusters, 40.0);
        assert_eq!(result.line_texts, vec!["aaaa", "aaaa", "aa"]);
        assert_eq!(result.spans.len(), 3);
        for span in &result.spans {
            assert!(span.width() <= 40.0);
        }
    }

    #[test]
    fn splits_unbreakable_run() {
        let clusters: Vec<GlyphCluster> = (0..6).map(|i| cluster(i, 20.0, false)).collect();
        let result = layout("abcdef", clusters, 80.0);
        assert_eq!(result.line_texts, vec!["abcd", "ef"]);
        assert_eq!(result.spans[0].width(), 80.0);
        assert_eq!(result.spans[1].width(), 40.0);
    }

    #[test]
    fn no_overhang_for_word_wider_than_line() {
        let result = layout(
            "hello supercalifragilistic",
            vec![
                cluster(0, 30.0, false),
                cluster(5, 5.0, true),
                cluster(6, 93.0, false),
            ],
            80.0,
        );
        assert_eq!(result.line_texts.len(), 2);
        assert_eq!(result.line_texts[0].trim_end(), "hello");
        assert_eq!(result.spans[0].width(), 35.0);
        assert_eq!(result.spans[1].width(), 93.0);
    }

    #[test]
    fn single_cluster_wider_than_line_stays_alone() {
        let result = layout("x", vec![cluster(0, 93.0, false)], 80.0);
        assert_eq!(result.line_texts, vec!["x"]);
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].width(), 93.0);
    }

    #[test]
    fn first_line_uses_narrower_space() {
        let clusters = vec![
            cluster(0, 20.0, false),
            cluster(2, 4.0, true),
            cluster(3, 20.0, false),
            cluster(5, 4.0, true),
            cluster(6, 20.0, false),
            cluster(8, 4.0, true),
            cluster(9, 20.0, false),
        ];
        let result = TextFlowLayouter::new(
            "aa bb cc dd".to_string(),
            TextStyle::default(),
            clusters,
            16.0,
        )
        .compute_layout(32.0, 64.0, (0.0, 0.0));
        let texts: Vec<String> = result
            .line_texts
            .iter()
            .map(|s| s.trim_end().to_string())
            .collect();
        assert_eq!(texts, vec!["aa", "bb cc", "dd"]);
        assert_eq!(result.spans[0].width(), 24.0);
        assert_eq!(result.spans[1].width(), 48.0);
        assert_eq!(result.spans[2].width(), 20.0);
    }
}
