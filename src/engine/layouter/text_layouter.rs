use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use ui_layout::{FlowLayoutContext, FlowLayouter, LayoutContext, LineSpan, MeasureResult};

use crate::engine::bridge::text::GlyphCluster;
use crate::engine::layouter::types::TextStyle;

thread_local! {
    static TEXT_RESULTS: RefCell<HashMap<usize, TextLayoutResult>> =
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

/// A self-layouting text object that implements [`FlowLayouter`].
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
        clusters: Vec<GlyphCluster>,
        line_height: f32,
    ) -> Self {
        let id = NEXT_TEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            id,
            text,
            clusters,
            line_height,
        }
    }

    /// Retrieve the layout result for `id` from the thread-local cache.
    pub fn get_result(id: usize) -> Option<TextLayoutResult> {
        TEXT_RESULTS.with(|cache| cache.borrow().get(&id).cloned())
    }

    fn remove_result(id: usize) {
        TEXT_RESULTS.with(|cache| {
            cache.borrow_mut().remove(&id);
        });
    }

    fn compute_layout(
        &self,
        available_inline_size: f32,
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

        // Resolved line width: available_inline_size, capped at default 80px if zero.
        let line_width = if available_inline_size > 0.0 {
            available_inline_size
        } else {
            // If the available size is zero (e.g. at the start of a layout pass
            // with no prior content), use the container's intrinsic size.
            // We conservatively use a large value so nothing wraps unexpectedly.
            f32::MAX
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
                    x_pos = start_pos.0;

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
                    x_pos = start_pos.0;
                    y_pos += lh;
                    line_index += 1;
                    accumulated = 0.0;
                    last_breakable_cluster = None;
                }

                i += 1;
                continue;
            }

            // Accumulate width
            accumulated += frag.width;

            if frag.break_allowed {
                last_breakable_cluster = Some(i + 1);
            }

            // Check if we need to break
            if accumulated > line_width {
                if let Some(break_at) = last_breakable_cluster {
                    let break_byte = if break_at < clusters.len() {
                        clusters[break_at].byte_offset
                    } else {
                        text_len
                    };

                    if break_byte > line_start || spans.is_empty() {
                        emit_line!(break_at, break_byte);

                        line_start = break_byte;
                        line_start_idx = break_at;
                        x_pos = start_pos.0;
                        y_pos += lh;
                        line_index += 1;
                        last_breakable_cluster = None;
                    }

                    if break_at <= i {
                        accumulated = clusters[break_at..=i].iter().map(|c| c.width).sum::<f32>();
                    } else {
                        accumulated = frag.width;
                    }

                    // If the break is after the current cluster, we've already
                    // included frag.width in accumulated; keep accumulated.
                    // If break_at == i+1 (break after current), accumulated is correct.
                } else {
                    // No breakable point found – this should not happen with normal text.
                    // Just accumulate and continue (or break at cluster boundary).
                    // If this is the last cluster, it will be emitted at the end.
                }
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

impl FlowLayouter for TextFlowLayouter {
    fn layout(&self, ctx: &FlowLayoutContext) -> Vec<LineSpan> {
        let result = self.compute_layout(ctx.available_inline_size, ctx.start_pos);
        let spans = result.spans.clone();

        TEXT_RESULTS.with(|cache| {
            cache.borrow_mut().insert(self.id, result);
        });

        spans
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
