use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ui_layout::{
    BoxModel, CustomLayouter, InlineBox, LayoutBox, LayoutContext, LineSpan, MeasureResult, Rect,
};

use crate::engine::bridge::text::GlyphCluster;
use crate::engine::layouter::builder::DEFAULT_LINE_FACTOR;
use crate::engine::layouter::types::{LineHeight, TextAlign, TextFlowStyle, WhiteSpace};

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
    flow_style: TextFlowStyle,
}

impl TextFlowLayouter {
    pub fn new(text: String, flow_style: TextFlowStyle, mut clusters: Vec<GlyphCluster>) -> Self {
        clusters.sort_by_key(|c| c.byte_offset);
        let id = NEXT_TEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            id,
            text,
            flow_style,
            clusters,
        }
    }

    /// Resolved line height in pixels, derived from the flow style.
    fn line_height(&self) -> f32 {
        match self.flow_style.line_height {
            LineHeight::Number(factor) => self.flow_style.font_size * factor,
            LineHeight::Normal => self.flow_style.font_size * DEFAULT_LINE_FACTOR,
            LineHeight::Px(px) => px,
        }
        .max(1.0)
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
        let lh = self.line_height();
        let text_len = self.text.len();
        let clusters = &self.clusters;

        let align = self.flow_style.text_align;
        let white_space = self.flow_style.white_space;
        let wrap_overflow = matches!(
            white_space,
            WhiteSpace::Normal
                | WhiteSpace::PreWrap
                | WhiteSpace::PreLine
                | WhiteSpace::BreakSpaces
        );
        let forced_newline = matches!(
            white_space,
            WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine | WhiteSpace::BreakSpaces
        );
        let split_unbreakable = matches!(white_space, WhiteSpace::Normal);

        // Edge-case: no clusters but non-empty text (e.g. spaces-only).
        if clusters.is_empty() && !self.text.is_empty() {
            if forced_newline && self.text.bytes().all(|b| b == b'\n') {
                // Every preserved newline is a segment break, so N newlines
                // produce N + 1 lines.
                let line_count = self.text.bytes().filter(|b| *b == b'\n').count() + 1;
                return TextLayoutResult {
                    spans: (0..line_count)
                        .map(|i| LineSpan {
                            x_range: start_pos.0..start_pos.0,
                            line_pos: (start_pos.0, start_pos.1 + i as f32 * lh),
                            line_index: i,
                        })
                        .collect(),
                    line_texts: vec![String::new(); line_count],
                };
            }
            return TextLayoutResult {
                spans: vec![LineSpan {
                    x_range: start_pos.0..start_pos.0,
                    line_pos: start_pos,
                    line_index: 0,
                }],
                line_texts: vec![self.text.clone()],
            };
        }
        if clusters.is_empty() && !self.text.is_empty() {
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
            0.0
        };
        let line_width = |line_index: usize| {
            if line_index == 0 {
                first_line_width
            } else {
                available_space
            }
        };

        let aligned_x = |line_index: usize, x_pos: f32, line_w: f32| {
            let available = line_width(line_index);

            x_pos
                + match align {
                    TextAlign::Left => 0.0,
                    TextAlign::Center => (available - line_w) / 2.0,
                    TextAlign::Right => available - line_w,
                }
        };

        let mut spans: Vec<LineSpan> = Vec::new();
        let mut line_texts: Vec<String> = Vec::new();

        let mut line_start: usize = 0; // byte offset where current line starts
        let mut x_pos = start_pos.0; // Actual line cordination
        let mut y_pos = start_pos.1;
        let mut line_index: usize = 0;
        let mut accumulated: f32 = 0.0; // Running width placed on the current line; used for wrap/overflow checks.
        let mut last_breakable_cluster: Option<usize> = None; // cluster index (exclusive)

        let clusters_between = |from_byte: usize, to_byte: usize| -> f32 {
            let from_idx = clusters.partition_point(|c| c.byte_offset < from_byte);
            let to_idx = clusters.partition_point(|c| c.byte_offset < to_byte);
            clusters[from_idx..to_idx].iter().map(|c| c.width).sum()
        };

        macro_rules! emit_line {
            ($end_byte:expr) => {{
                let end_byte = $end_byte;

                let line_w = clusters_between(line_start, end_byte);
                x_pos = aligned_x(line_index, x_pos, line_w);

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

            if forced_newline && let Some(rel) = self.text[line_start..next_byte].find('\n') {
                let nl_byte = line_start + rel;
                let nl_before_cluster = nl_byte < frag.byte_offset;

                if nl_byte > line_start {
                    emit_line!(nl_byte);
                } else {
                    // Empty line (e.g. consecutive newlines)
                    x_pos = aligned_x(line_index, x_pos, 0.0);
                    spans.push(LineSpan {
                        x_range: x_pos..x_pos,
                        line_pos: (x_pos, y_pos),
                        line_index,
                    });
                    line_texts.push(String::new());
                }

                line_start = nl_byte + 1;
                x_pos = start_pos.0;
                y_pos += lh;
                line_index += 1;
                accumulated = 0.0;
                last_breakable_cluster = None;

                // A newline before this cluster's glyph means the cluster
                // starts the new line, so re-process it below.
                if !nl_before_cluster {
                    i += 1;
                }
                continue;
            }

            // Check if placing this cluster would overflow the line.
            let current_line_width = line_width(line_index);
            if wrap_overflow && accumulated > 0.0 && accumulated + frag.width > current_line_width {
                if let Some(break_at) = last_breakable_cluster {
                    // Break at the last known breakable cluster (word boundary).
                    let break_byte = if break_at < clusters.len() {
                        clusters[break_at].byte_offset
                    } else {
                        text_len
                    };

                    if break_byte > line_start || spans.is_empty() {
                        emit_line!(break_byte);

                        line_start = break_byte;
                        x_pos = start_pos.0;
                        y_pos += lh;
                        line_index += 1;
                        last_breakable_cluster = None;

                        // Carry over the width of any non-breakable clusters
                        // that move to the new line together with this one.
                        accumulated = if break_at < i {
                            clusters_between(break_byte, clusters[i].byte_offset)
                        } else {
                            0.0
                        };
                    }
                } else if accumulated + frag.width <= available_space {
                    // The current line holds a single unbreakable run and the
                    // next line is wide enough to take the whole word: move it
                    // there instead of splitting it mid-word. This keeps the
                    // first word intact when the first line is narrower than
                    // the following ones.
                    y_pos += lh;
                    line_index += 1;
                    x_pos = start_pos.0;
                    accumulated = clusters_between(line_start, clusters[i].byte_offset);
                    last_breakable_cluster = None;
                } else if split_unbreakable {
                    // Unbreakable run that is wider than the next line as well:
                    // split at the current cluster boundary.
                    let break_byte = clusters[i].byte_offset;
                    if break_byte > line_start || spans.is_empty() {
                        emit_line!(break_byte);

                        line_start = break_byte;
                        x_pos = start_pos.0;
                        y_pos += lh;
                        line_index += 1;
                        last_breakable_cluster = None;
                        accumulated = 0.0;
                    }
                }
            }

            // Accumulate width
            accumulated += frag.width;

            if frag.break_allowed {
                last_breakable_cluster = Some(i + 1);
            }

            i += 1;
        }

        // Emit the final line(s). For preserved newlines we split the remaining
        // text exactly on '\n', so every newline yields a line and a trailing
        // newline adds one final empty line — identical to `str::split('\n')`.
        // The main loop above already emitted a line for each *internal*
        // newline; this pass emits the current line plus any lines opened by
        // trailing newlines, so newline tail handling lives in a single place.
        if forced_newline {
            let rest = &self.text[line_start..text_len];
            if rest.is_empty() {
                // The text ended on a newline (e.g. "abc\n"): split('\n') still
                // yields one trailing empty line. This line follows a break, so
                // its coordinate origin is the box left edge (x_pos = 0).
                x_pos = aligned_x(line_index, 0.0, 0.0);
                spans.push(LineSpan {
                    x_range: x_pos..x_pos,
                    line_pos: (x_pos, y_pos),
                    line_index,
                });
                line_texts.push(String::new());
            } else {
                for seg in rest.split('\n') {
                    let seg_end = line_start + seg.len();
                    let line_w = clusters_between(line_start, seg_end);
                    // The first segment uses the incoming `x_pos` (the box
                    // origin for the first line, or 0 after a prior break); every
                    // later segment is an empty line following a break, so its
                    // origin is the box left edge. Resetting `x_pos` here keeps
                    // the carry-over value from accumulating across segments.
                    x_pos = aligned_x(line_index, x_pos, line_w);
                    spans.push(LineSpan {
                        x_range: x_pos..(x_pos + line_w),
                        line_pos: (x_pos, y_pos),
                        line_index,
                    });
                    line_texts.push(seg.to_string());
                    line_index += 1;
                    y_pos += lh;
                    line_start = seg_end + 1; // step over the '\n'
                    x_pos = start_pos.0;
                }
            }
        } else if line_start < text_len {
            let line_w = clusters_between(line_start, text_len);
            let trimmed = &self.text[line_start..text_len];
            x_pos = aligned_x(line_index, x_pos, line_w);

            spans.push(LineSpan {
                x_range: x_pos..(x_pos + line_w),
                line_pos: (x_pos, y_pos),
                line_index,
            });
            line_texts.push(trimmed.trim_end_matches('\n').to_string());
        }

        if spans.is_empty() {
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
        let lh = self.line_height();
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
        let total_height = self.line_height();
        MeasureResult {
            width: total_width,
            height: total_height,
        }
    }

    fn write_debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextFlowLayouter [{}]", self.text.escape_debug())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::engine::layouter::types::TextFlowStyle;

    fn cluster(byte_offset: usize, width: f32, break_allowed: bool) -> GlyphCluster {
        GlyphCluster {
            byte_offset,
            width,
            break_allowed,
        }
    }

    fn layout(text: &str, clusters: Vec<GlyphCluster>, line_width: f32) -> TextLayoutResult {
        TextFlowLayouter::new(text.to_string(), TextFlowStyle::default(), clusters).compute_layout(
            line_width,
            line_width,
            (0.0, 0.0),
        )
    }

    fn layout_with(
        text: &str,
        clusters: Vec<GlyphCluster>,
        line_width: f32,
        white_space: WhiteSpace,
    ) -> TextLayoutResult {
        let mut flow = TextFlowStyle::default();
        flow.white_space = white_space;
        TextFlowLayouter::new(text.to_string(), flow, clusters).compute_layout(
            line_width,
            line_width,
            (0.0, 0.0),
        )
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
    fn aligns_each_line_within_the_available_inline_space() {
        let clusters = vec![
            cluster(0, 20.0, false),
            cluster(2, 10.0, true),
            cluster(3, 20.0, false),
        ];
        let mut centered = TextFlowStyle::default();
        centered.text_align = TextAlign::Center;
        let result = TextFlowLayouter::new("aa bb".to_string(), centered, clusters.clone())
            .compute_layout(40.0, 40.0, (0.0, 0.0));
        assert_eq!(result.line_texts, vec!["aa ", "bb"]);
        assert_eq!(result.spans[0].line_pos.0, 5.0);
        assert_eq!(result.spans[1].line_pos.0, 10.0);
        assert_eq!(result.spans[0].x_range, 5.0..35.0);
        assert_eq!(result.spans[1].x_range, 10.0..30.0);

        let mut right = TextFlowStyle::default();
        right.text_align = TextAlign::Right;
        let result = TextFlowLayouter::new("aa".to_string(), right, clusters[..1].to_vec())
            .compute_layout(40.0, 40.0, (0.0, 0.0));
        assert_eq!(result.spans[0].line_pos.0, 20.0);
        assert_eq!(result.spans[0].x_range, 20.0..40.0);
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
            TextFlowStyle::default(),
            clusters,
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

    #[test]
    fn first_word_moves_to_next_line_when_first_line_is_narrow() {
        let clusters = vec![
            cluster(0, 9.0, false),
            cluster(1, 9.0, false),
            cluster(2, 9.0, false),
            cluster(3, 9.0, false),
            cluster(4, 9.0, false),
            cluster(5, 4.0, true),
            cluster(6, 10.0, false),
            cluster(7, 10.0, false),
            cluster(8, 10.0, false),
            cluster(9, 10.0, false),
            cluster(10, 10.0, false),
        ];
        let result = TextFlowLayouter::new(
            "Hello world".to_string(),
            TextFlowStyle::default(),
            clusters,
        )
        .compute_layout(30.0, 100.0, (0.0, 0.0));
        assert_eq!(result.line_texts, vec!["Hello world"]);
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].line_index, 1);
        assert_eq!(result.spans[0].width(), 99.0);
    }

    #[test]
    fn first_word_splits_when_too_wide_for_every_line() {
        let clusters = vec![
            cluster(0, 9.0, false),
            cluster(1, 9.0, false),
            cluster(2, 9.0, false),
            cluster(3, 9.0, false),
            cluster(4, 9.0, false),
        ];
        let result = TextFlowLayouter::new("Hello".to_string(), TextFlowStyle::default(), clusters)
            .compute_layout(30.0, 40.0, (0.0, 0.0));
        assert_eq!(result.line_texts, vec!["Hell", "o"]);
    }

    #[test]
    fn nowrap_never_wraps_on_overflow() {
        let clusters = vec![
            cluster(0, 30.0, false),
            cluster(3, 5.0, true),
            cluster(4, 30.0, false),
            cluster(7, 5.0, true),
            cluster(8, 30.0, false),
        ];
        let result = layout_with("aaa bbb ccc", clusters, 50.0, WhiteSpace::Nowrap);
        assert_eq!(result.line_texts, vec!["aaa bbb ccc"]);
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].width(), 100.0);
    }

    #[test]
    fn pre_breaks_only_on_newline() {
        let no_wrap_clusters = vec![
            cluster(0, 40.0, false),
            cluster(3, 4.0, true),
            cluster(4, 40.0, false),
            cluster(7, 4.0, true),
            cluster(8, 40.0, false),
        ];
        let no_wrap = layout_with("aaa bbb ccc", no_wrap_clusters, 60.0, WhiteSpace::Pre);
        assert_eq!(no_wrap.line_texts, vec!["aaa bbb ccc"]);
        assert_eq!(no_wrap.spans.len(), 1);
        assert_eq!(no_wrap.spans[0].width(), 128.0);

        let newline_clusters = vec![
            cluster(0, 40.0, false),
            cluster(4, 4.0, true),
            cluster(5, 40.0, false),
        ];
        let forced = layout_with("aaaa\nbbbb", newline_clusters, 50.0, WhiteSpace::Pre);
        assert_eq!(forced.line_texts, vec!["aaaa", "bbbb"]);
        assert_eq!(forced.spans.len(), 2);
        assert_eq!(forced.spans[0].line_index, 0);
        assert_eq!(forced.spans[1].line_index, 1);
    }

    #[test]
    fn pre_wrap_breaks_on_newline_and_wraps() {
        let clusters = vec![
            cluster(0, 20.0, false),
            cluster(2, 4.0, true),
            cluster(3, 20.0, false),
            cluster(5, 4.0, true),
            cluster(6, 20.0, false),
        ];
        let result = layout_with("aa bb cc", clusters, 34.0, WhiteSpace::PreWrap);
        let texts: Vec<String> = result
            .line_texts
            .iter()
            .map(|s| s.trim_end().to_string())
            .collect();
        assert_eq!(texts, vec!["aa", "bb", "cc"]);
    }

    #[test]
    fn pre_line_breaks_on_newline_and_wraps() {
        let clusters = vec![
            cluster(0, 20.0, false),
            cluster(2, 4.0, true),
            cluster(3, 20.0, false),
            cluster(5, 4.0, true),
            cluster(6, 20.0, false),
        ];
        let result = layout_with("aa bb cc", clusters, 34.0, WhiteSpace::PreLine);
        let texts: Vec<String> = result
            .line_texts
            .iter()
            .map(|s| s.trim_end().to_string())
            .collect();
        assert_eq!(texts, vec!["aa", "bb", "cc"]);
        assert_eq!(result.spans.len(), 3);
    }

    #[test]
    fn break_spaces_wraps_inside_whitespace_run() {
        let clusters = vec![
            cluster(0, 20.0, false),
            cluster(2, 5.0, true),
            cluster(3, 5.0, true),
            cluster(4, 20.0, false),
            cluster(5, 20.0, false),
        ];
        let result = layout_with("aa  bb", clusters, 25.0, WhiteSpace::BreakSpaces);
        assert_eq!(result.line_texts, vec!["aa ", " ", "bb"]);
        assert_eq!(result.spans.len(), 3);
    }

    #[test]
    fn leading_newline_emits_empty_first_line() {
        let clusters = vec![
            cluster(1, 10.0, false),
            cluster(2, 10.0, false),
            cluster(3, 10.0, false),
        ];
        let result = layout_with("\nabc", clusters, 100.0, WhiteSpace::PreWrap);
        assert_eq!(result.line_texts, vec!["", "abc"]);
        assert_eq!(result.spans.len(), 2);
        assert_eq!(result.spans[0].line_index, 0);
        assert_eq!(result.spans[0].width(), 0.0);
        assert_eq!(result.spans[1].line_index, 1);
        assert_eq!(result.spans[1].width(), 30.0);
    }

    #[test]
    fn trailing_newline_emits_empty_last_line() {
        let clusters = vec![
            cluster(0, 10.0, false),
            cluster(1, 10.0, false),
            cluster(2, 10.0, false),
        ];
        let result = layout_with("abc\n", clusters, 100.0, WhiteSpace::PreWrap);
        // "abc\n" has one preserved segment break → "abc" plus an empty line.
        assert_eq!(result.line_texts, vec!["abc", ""]);
        assert_eq!(result.spans.len(), 2);
        assert_eq!(result.spans[1].width(), 0.0);
    }

    #[test]
    fn leading_and_trailing_newline_emit_empty_lines() {
        let clusters = vec![
            cluster(1, 10.0, false),
            cluster(2, 10.0, false),
            cluster(3, 10.0, false),
        ];
        let result = layout_with("\nabc\n", clusters, 100.0, WhiteSpace::PreWrap);
        assert_eq!(result.line_texts, vec!["", "abc", ""]);
        assert_eq!(result.spans.len(), 3);
    }

    #[test]
    fn multiple_trailing_newlines_emit_empty_lines() {
        let clusters = vec![
            cluster(0, 10.0, false),
            cluster(1, 10.0, false),
            cluster(2, 10.0, false),
        ];
        let result = layout_with("abc\n\n", clusters, 100.0, WhiteSpace::PreWrap);
        assert_eq!(result.line_texts, vec!["abc", "", ""]);
        assert_eq!(result.spans.len(), 3);
    }

    #[test]
    fn all_newlines_emit_n_plus_one_lines() {
        // No glyph clusters: every preserved newline is a segment break, so
        // N newlines produce N + 1 lines.
        let result = layout_with("\n\n", vec![], 100.0, WhiteSpace::PreWrap);
        assert_eq!(result.line_texts, vec!["", "", ""]);
        assert_eq!(result.spans.len(), 3);
    }

    #[test]
    fn trailing_newline_ignored_for_normal_whitespace() {
        let clusters = vec![
            cluster(0, 10.0, false),
            cluster(1, 10.0, false),
            cluster(2, 10.0, false),
        ];
        // For Normal whitespace a trailing newline is collapsed, not a forced
        // break, so no extra empty line is emitted.
        let result = layout_with("abc\n", clusters, 100.0, WhiteSpace::Normal);
        assert_eq!(result.line_texts, vec!["abc"]);
        assert_eq!(result.spans.len(), 1);
    }

    #[test]
    fn line_after_newline_starts_at_left_edge() {
        // Every visual line of a text node must start at the box's left edge
        // (start_pos.x). With a zero origin this is x:0.
        let clusters = vec![
            cluster(0, 10.0, false),
            cluster(1, 10.0, false),
            cluster(2, 10.0, false),
            cluster(4, 10.0, false),
            cluster(5, 10.0, false),
            cluster(6, 10.0, false),
        ];
        let mut flow = TextFlowStyle::default();
        flow.white_space = WhiteSpace::PreWrap;
        let result = TextFlowLayouter::new("abc\ndef".to_string(), flow, clusters).compute_layout(
            100.0,
            100.0,
            (0.0, 0.0),
        );
        assert_eq!(result.spans.len(), 2);
        assert_eq!(result.spans[0].line_pos.0, 0.0, "first line x");
        assert_eq!(result.spans[1].line_pos.0, 0.0, "line after newline x");
    }

    #[test]
    fn line_after_newline_keeps_block_left_when_indented() {
        // When the text node is positioned away from the origin (e.g. an
        // indented/positioned block), every visual line must start at the box
        // left edge (start_pos.x), not at the absolute origin.
        let clusters = vec![
            cluster(0, 10.0, false),
            cluster(1, 10.0, false),
            cluster(2, 10.0, false),
            cluster(4, 10.0, false),
            cluster(5, 10.0, false),
            cluster(6, 10.0, false),
        ];
        let mut flow = TextFlowStyle::default();
        flow.white_space = WhiteSpace::PreWrap;
        let result = TextFlowLayouter::new("abc\ndef".to_string(), flow, clusters).compute_layout(
            100.0,
            100.0,
            (50.0, 0.0),
        );
        assert_eq!(result.spans.len(), 2);
        assert_eq!(result.spans[0].line_pos.0, 50.0, "first line x");
        assert_eq!(result.spans[1].line_pos.0, 50.0, "line after newline x");
    }

    #[test]
    fn wrapped_line_starts_at_left_edge() {
        // A width-triggered wrap (no newline) must also resume at the box left
        // edge.
        let clusters = vec![
            cluster(0, 30.0, false),
            cluster(3, 5.0, true),
            cluster(4, 30.0, false),
        ];
        let result =
            TextFlowLayouter::new("aaa bbb".to_string(), TextFlowStyle::default(), clusters)
                .compute_layout(40.0, 40.0, (50.0, 0.0));
        assert_eq!(result.spans.len(), 2);
        assert_eq!(result.spans[0].line_pos.0, 50.0, "first line x");
        assert_eq!(result.spans[1].line_pos.0, 50.0, "wrapped line x");
    }

    #[test]
    fn consecutive_newlines_emit_empty_lines() {
        let clusters = vec![
            cluster(0, 10.0, false),
            cluster(1, 10.0, false),
            cluster(4, 10.0, false),
            cluster(5, 10.0, false),
        ];
        let result = layout_with("ab\n\ncd", clusters, 100.0, WhiteSpace::PreWrap);
        assert_eq!(result.line_texts, vec!["ab", "", "cd"]);
        assert_eq!(result.spans.len(), 3);
        assert_eq!(result.spans[0].width(), 20.0);
        assert_eq!(result.spans[1].width(), 0.0);
        assert_eq!(result.spans[2].width(), 20.0);
    }

    #[test]
    fn newline_between_paragraphs_keeps_span_width() {
        let clusters = vec![
            cluster(0, 15.0, false),
            cluster(1, 15.0, false),
            cluster(3, 15.0, false),
        ];
        let result = layout_with("ab\nc", clusters, 100.0, WhiteSpace::Pre);
        assert_eq!(result.line_texts, vec!["ab", "c"]);
        assert_eq!(result.spans[0].width(), 30.0);
        assert_eq!(result.spans[1].width(), 15.0);
    }

    #[test]
    fn pre_wrap_does_not_split_unbreakable_word() {
        let clusters: Vec<GlyphCluster> = (0..6).map(|i| cluster(i, 20.0, false)).collect();
        let result = layout_with("abcdef", clusters, 80.0, WhiteSpace::PreWrap);
        assert_eq!(result.line_texts, vec!["abcdef"]);
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].width(), 120.0);
    }

    #[test]
    fn pre_line_does_not_split_unbreakable_word() {
        let clusters: Vec<GlyphCluster> = (0..6).map(|i| cluster(i, 20.0, false)).collect();
        let result = layout_with("abcdef", clusters, 80.0, WhiteSpace::PreLine);
        assert_eq!(result.line_texts, vec!["abcdef"]);
        assert_eq!(result.spans.len(), 1);
    }

    #[test]
    fn break_spaces_does_not_split_unbreakable_word() {
        let clusters: Vec<GlyphCluster> = (0..6).map(|i| cluster(i, 20.0, false)).collect();
        let result = layout_with("abcdef", clusters, 80.0, WhiteSpace::BreakSpaces);
        assert_eq!(result.line_texts, vec!["abcdef"]);
        assert_eq!(result.spans.len(), 1);
    }

    #[test]
    fn pre_wrap_wraps_at_whitespace_but_not_inside_word() {
        let clusters = vec![
            cluster(0, 20.0, false),
            cluster(1, 20.0, false),
            cluster(2, 20.0, false),
            cluster(3, 20.0, false),
            cluster(4, 4.0, true),
            cluster(5, 15.0, false),
            cluster(6, 15.0, false),
        ];
        let result = layout_with("abcd ef", clusters, 70.0, WhiteSpace::PreWrap);
        let texts: Vec<String> = result
            .line_texts
            .iter()
            .map(|s| s.trim_end().to_string())
            .collect();
        assert_eq!(texts, vec!["abcd", "ef"]);
    }

    #[test]
    fn empty_text_emits_one_empty_line() {
        let result = layout("", vec![], 100.0);
        assert_eq!(result.line_texts, vec![""]);
        assert_eq!(result.spans.len(), 1);
    }

    #[test]
    fn trailing_newlines_match_split_semantics() {
        // The whole tail is owned by a single split('\n') pass, so the line
        // count matches `str::split('\n')` exactly.
        let cases: &[(&str, &[&str])] = &[
            ("foo", &["foo"]),
            ("foo\n", &["foo", ""]),
            ("foo\n\n", &["foo", "", ""]),
            ("\n", &["", ""]),
            ("\n\n", &["", "", ""]),
        ];
        for (text, expected) in cases {
            let clusters: Vec<GlyphCluster> = text
                .chars()
                .enumerate()
                .filter(|(_, c)| !c.is_whitespace())
                .map(|(i, _)| cluster(i, 10.0, false))
                .collect();
            let result = layout_with(text, clusters, 100.0, WhiteSpace::Pre);
            assert_eq!(&result.line_texts[..], *expected, "text = {:?}", text);
        }
    }

    #[test]
    fn trailing_newlines_do_not_accumulate_x_pos() {
        // Multiple trailing newlines under center alignment must place every
        // empty line at the same centered x. The carry-over `x_pos` must reset
        // to 0 between segments so it does not accumulate across them.
        let clusters = vec![cluster(0, 10.0, false), cluster(1, 10.0, false)];
        let mut flow = TextFlowStyle::default();
        flow.white_space = WhiteSpace::Pre;
        flow.text_align = TextAlign::Center;
        let result = TextFlowLayouter::new("ab\n\n".to_string(), flow, clusters).compute_layout(
            100.0,
            100.0,
            (0.0, 0.0),
        );

        assert_eq!(result.line_texts, vec!["ab", "", ""]);
        // First line "ab" is centered: (100 - 20) / 2 == 40.
        assert_eq!(result.spans[0].line_pos.0, 40.0);
        // Both trailing empty lines sit at the centered x for zero width (50),
        // and crucially they share the same x — no accumulation.
        assert_eq!(result.spans[1].line_pos.0, 50.0);
        assert_eq!(result.spans[2].line_pos.0, 50.0);
    }
}
