//! Cache benchmarks.
//!
//! * Compares the pre-LRU data structures (linear-scan layout cache) against
//!   the current LRU-based implementations.
//! * Quantifies what a shaping-cache hit saves compared to real text shaping.
//! * Measures the HTTP response cache hit/miss paths.
//! * Measures the selector match cache lookup and eviction paths.
//! * Compares `normalize_whitespace` against its pre-optimization form.
//!
//! Run with `cargo bench --bench cache_bench`.

use std::borrow::Cow;
use std::hint::black_box;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use lru::LruCache;
use orinium_text::{FontSystem, TextLayouter, TextStyle};
use smol_str::SmolStr;

use orinium_browser::engine::css::parser::ComplexSelector;
use orinium_browser::engine::layouter::match_cache::SelectorMatchCache;
use orinium_browser::engine::layouter::normalize_whitespace;
use orinium_browser::engine::layouter::types::WhiteSpace;
use orinium_browser::platform::network::{Cache, Response, StatusCode};
use orinium_browser::platform::renderer::text_cache::TextShapeCache;

/// Working set of the layout cache: mirrors `LAYOUT_CACHE_CAPACITY`.
const LAYOUT_CACHE_CAPACITY: usize = 128;
/// Lookups performed per benchmark iteration.
const OPS_PER_ITER: usize = 32;
/// Share of lookups that hit (percent).
const HIT_RATIO_PERCENT: usize = 90;

fn tune(group: &mut criterion::BenchmarkGroup<criterion::measurement::WallTime>) {
    group
        .measurement_time(Duration::from_secs(3))
        .warm_up_time(Duration::from_secs(1));
}

/// A realistic sentence mixed with CJK so shaping does real work.
const SAMPLE_TEXT: &str =
    "The quick brown fox jumps over the lazy dog. 敏い茶色の狐が怠惰な犬を飛び越える。";

// ---------------------------------------------------------------------------
// shaping: uncached vs. cache hit
// ---------------------------------------------------------------------------

fn bench_shaping(c: &mut Criterion) {
    let mut group = c.benchmark_group("shaping");
    tune(&mut group);

    let style = TextStyle::default();

    group.bench_function("shape_text_uncached", |b| {
        let mut fs = FontSystem::new();
        if fs.db.is_empty() {
            eprintln!("warning: no system fonts found, shaping results are not representative");
        }
        b.iter(|| {
            let mut layouter = TextLayouter::new();
            black_box(layouter.shape_text(&mut fs, black_box(SAMPLE_TEXT), &style))
        });
    });

    group.bench_function("shape_cache_hit", |b| {
        let mut fs = FontSystem::new();
        let mut layouter = TextLayouter::new();
        let shaped = Arc::new(layouter.shape_text(&mut fs, SAMPLE_TEXT, &style));

        let cache = TextShapeCache::with_capacity(1024);
        cache.insert(SAMPLE_TEXT, &style, Arc::clone(&shaped));

        b.iter(|| black_box(cache.get(black_box(SAMPLE_TEXT), &style)));
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// TextShapeCache: hit throughput and insert-with-eviction path
// ---------------------------------------------------------------------------

fn bench_text_shape_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_shape_cache");
    tune(&mut group);

    let style = TextStyle::default();
    let mut fs = FontSystem::new();
    let mut layouter = TextLayouter::new();
    let shaped = Arc::new(layouter.shape_text(&mut fs, SAMPLE_TEXT, &style));

    let keys: Vec<String> = (0..1024)
        .map(|i| format!("cached label number {i}"))
        .collect();

    group.bench_function("hit_1024_working_set", |b| {
        let cache = TextShapeCache::with_capacity(4096);
        for key in &keys {
            cache.insert(key, &style, Arc::clone(&shaped));
        }

        let mut i = 0usize;
        b.iter(|| {
            let key = &keys[i % keys.len()];
            i += 1;
            black_box(cache.get(key, &style))
        });
    });

    group.bench_function("insert_with_eviction", |b| {
        let cache = TextShapeCache::with_capacity(256);
        let mut i = 0usize;
        b.iter(|| {
            i += 1;
            cache.insert(&format!("unique text {i}"), &style, Arc::clone(&shaped));
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// layout cache: pre-LRU Vec scan vs. current LruCache
// ---------------------------------------------------------------------------

/// Fields of a lookup key, rebuilt by the caller on every access (the
/// production code clones `font_families` on each call).
#[derive(Clone, PartialEq, Eq)]
struct LookupFields {
    font_size_bits: u32,
    color: u32,
    font_weight: u16,
    font_style: u8,
    line_height: u8,
    font_families: Vec<String>,
}

#[derive(Clone)]
struct LayoutValue(Arc<Vec<u32>>);

fn sample_value(seed: usize) -> LayoutValue {
    LayoutValue(Arc::new(vec![seed as u32; 256]))
}

/// Mirrors the pre-LRU layout cache: a `Vec` scanned linearly, entries moved
/// to the front on hit and popped from the tail beyond capacity.
#[derive(Default)]
struct VecScanCache {
    entries: Vec<VecScanEntry>,
}

struct VecScanEntry {
    text: String,
    fields: LookupFields,
    value: LayoutValue,
}

impl VecScanCache {
    fn get(&mut self, text: &str, fields: &LookupFields) -> Option<LayoutValue> {
        let pos = self
            .entries
            .iter()
            .position(|cached| cached.text == text && cached.fields == *fields)?;
        let entry = self.entries.remove(pos);
        let value = entry.value.clone();
        self.entries.insert(0, entry);
        Some(value)
    }

    fn put(&mut self, text: &str, fields: &LookupFields, value: LayoutValue) {
        self.entries.insert(
            0,
            VecScanEntry {
                text: text.to_string(),
                fields: fields.clone(),
                value,
            },
        );
        if self.entries.len() > LAYOUT_CACHE_CAPACITY {
            self.entries.pop();
        }
    }
}

/// Mirrors the current layout cache: `lru::LruCache` keyed by a hashable
/// struct (`SmolStr` text + cloned font families), as in `text_renderer.rs`.
struct LruLayoutCache {
    inner: LruCache<LayoutCacheKey, LayoutValue>,
}

#[derive(Clone, Eq, PartialEq, Hash)]
struct LayoutCacheKey {
    text: SmolStr,
    font_size_bits: u32,
    color: u32,
    font_weight: u16,
    font_style: u8,
    line_height: u8,
    font_families: Vec<String>,
}

impl LruLayoutCache {
    fn new() -> Self {
        Self {
            inner: LruCache::new(NonZeroUsize::new(LAYOUT_CACHE_CAPACITY).unwrap()),
        }
    }

    fn key(text: &str, fields: &LookupFields) -> LayoutCacheKey {
        LayoutCacheKey {
            text: SmolStr::new(text),
            font_size_bits: fields.font_size_bits,
            color: fields.color,
            font_weight: fields.font_weight,
            font_style: fields.font_style,
            line_height: fields.line_height,
            font_families: fields.font_families.clone(),
        }
    }

    fn get(&mut self, text: &str, fields: &LookupFields) -> Option<LayoutValue> {
        self.inner.get(&Self::key(text, fields)).cloned()
    }

    fn put(&mut self, text: &str, fields: &LookupFields, value: LayoutValue) {
        self.inner.put(Self::key(text, fields), value);
    }
}

enum Op {
    Hit(usize),
    Miss(usize),
}

/// 90% hits over the working set, 10% misses with fresh keys.
fn workload() -> Vec<Op> {
    (0..OPS_PER_ITER)
        .map(|op| {
            if op % 100 < HIT_RATIO_PERCENT {
                Op::Hit((op * 7) % LAYOUT_CACHE_CAPACITY)
            } else {
                Op::Miss(op)
            }
        })
        .collect()
}

fn working_set_fields() -> Vec<LookupFields> {
    (0..LAYOUT_CACHE_CAPACITY)
        .map(|i| LookupFields {
            font_size_bits: (12.0 + (i % 12) as f32).to_bits(),
            color: 0xFF00_0000 | i as u32,
            font_weight: 400 + (i % 300) as u16,
            font_style: (i % 3) as u8,
            line_height: (i % 2) as u8,
            font_families: vec![
                "Inter".to_string(),
                "Noto Sans".to_string(),
                format!("Family{i}"),
            ],
        })
        .collect()
}

fn working_set_texts() -> Vec<String> {
    (0..LAYOUT_CACHE_CAPACITY)
        .map(|i| format!("Working set label number {i}"))
        .collect()
}

fn bench_layout_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("layout_cache");
    tune(&mut group);

    let texts = working_set_texts();
    let fields = working_set_fields();
    let ops = workload();

    group.bench_function("vec_scan_baseline", |b| {
        b.iter_batched_ref(
            || {
                let mut cache = VecScanCache::default();
                for (i, text) in texts.iter().enumerate() {
                    cache.put(text, &fields[i], sample_value(i));
                }
                cache
            },
            |cache| {
                for op in &ops {
                    match op {
                        Op::Hit(idx) => {
                            black_box(cache.get(&texts[*idx], &fields[*idx]).map(|v| v.0.len()));
                        }
                        Op::Miss(n) => {
                            let text = format!("Miss label {n}");
                            if cache.get(&text, &fields[0]).is_none() {
                                cache.put(&text, &fields[0], sample_value(*n));
                            }
                        }
                    }
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.bench_function("lru_current", |b| {
        b.iter_batched_ref(
            || {
                let mut cache = LruLayoutCache::new();
                for (i, text) in texts.iter().enumerate() {
                    cache.put(text, &fields[i], sample_value(i));
                }
                cache
            },
            |cache| {
                for op in &ops {
                    match op {
                        Op::Hit(idx) => {
                            black_box(cache.get(&texts[*idx], &fields[*idx]).map(|v| v.0.len()));
                        }
                        Op::Miss(n) => {
                            let text = format!("Miss label {n}");
                            if cache.get(&text, &fields[0]).is_none() {
                                cache.put(&text, &fields[0], sample_value(*n));
                            }
                        }
                    }
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// selector match cache
// ---------------------------------------------------------------------------

fn bench_selector_match_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("selector_match_cache");
    tune(&mut group);

    // One allocation per selector mirrors the rule set, where every rule group
    // owns an `Arc<ComplexSelector>` that stays alive for the whole epoch.
    let selectors: Vec<Arc<ComplexSelector>> = (0..512)
        .map(|_| Arc::new(ComplexSelector { parts: Vec::new() }))
        .collect();
    let node_count = 128u32;

    group.bench_function("hit_512_selectors_x_128_nodes", |b| {
        let mut cache = SelectorMatchCache::new();
        for (s_idx, selector) in selectors.iter().enumerate() {
            for node in 0..node_count {
                cache.get_or_insert(selector, node, || (s_idx as u32 + node).is_multiple_of(2));
            }
        }

        let mut i = 0usize;
        b.iter(|| {
            let selector = &selectors[i % selectors.len()];
            let node = (i / selectors.len()) as u32 % node_count;
            i += 1;
            black_box(
                cache.get_or_insert(black_box(selector), black_box(node), || {
                    unreachable!("a warm cache must not recompute the match")
                }),
            )
        });
    });

    group.bench_function("insert_over_capacity", |b| {
        let mut cache = SelectorMatchCache::with_capacity(4096);
        let mut i = 0usize;
        b.iter(|| {
            i += 1;
            let selector = &selectors[i % selectors.len()];
            let node = (i / selectors.len()) as u32;
            black_box(cache.get_or_insert(selector, node, || true))
        });
    });

    group.bench_function("over_capacity_second_pass", |b| {
        let mut cache = SelectorMatchCache::with_capacity(4096);
        for (s_idx, selector) in selectors.iter().enumerate() {
            for node in 0..node_count {
                cache.get_or_insert(selector, node, || (s_idx as u32 + node).is_multiple_of(2));
            }
        }
        // Fill beyond capacity with different keys to create skips.
        for (s_idx, selector) in selectors.iter().enumerate() {
            for node in node_count..(node_count * 2) {
                cache.get_or_insert(selector, node, || (s_idx as u32 + node).is_multiple_of(2));
            }
        }

        let mut i = 0usize;
        b.iter(|| {
            let selector = &selectors[i % selectors.len()];
            let node = (i / selectors.len()) as u32 % node_count;
            i += 1;
            black_box(
                cache.get_or_insert(black_box(selector), black_box(node), || {
                    unreachable!("entries admitted on pass 1 must remain cached")
                }),
            )
        });
    });

    // Baseline for the hit path: a plain `HashMap` keyed the same way, without
    // the LRU recency bookkeeping. Each `(selector, element)` pair is queried
    // exactly once per build, so the move-to-front that `LruCache` performs on
    // every hit is pure overhead within an epoch.
    group.bench_function("hashmap_hit", |b| {
        let mut cache: std::collections::HashMap<(usize, u32), bool> =
            std::collections::HashMap::new();
        for (s_idx, selector) in selectors.iter().enumerate() {
            for node in 0..node_count {
                cache.insert(
                    (Arc::as_ptr(selector) as usize, node),
                    (s_idx as u32 + node).is_multiple_of(2),
                );
            }
        }

        let mut i = 0usize;
        b.iter(|| {
            let key = (
                Arc::as_ptr(&selectors[i % selectors.len()]) as usize,
                (i / selectors.len()) as u32 % node_count,
            );
            i += 1;
            black_box(*cache.get(black_box(&key)).unwrap())
        });
    });

    // Production-scale hit pass: ~1500 selectors x ~40 candidates each is
    // about the 60k pair count a large page produces. The traversal is
    // element-major: for each element, walk its candidate groups (matching the
    // builder's actual access order), then repeat the whole pass as the second
    // layout build does.
    group.bench_function("hit_production_scale_60k", |b| {
        let selector_count = 1500usize;
        let per_element = 40usize;
        let big_selectors: Vec<Arc<ComplexSelector>> = (0..selector_count)
            .map(|_| Arc::new(ComplexSelector { parts: Vec::new() }))
            .collect();
        let elements = 60_000usize / per_element;

        let mut cache = SelectorMatchCache::new();
        let mut keys = Vec::with_capacity(60_000);
        for element in 0..elements {
            for k in 0..per_element {
                let sel = (element * 7 + k * 13) % selector_count;
                keys.push((big_selectors[sel].clone(), element as u32));
            }
        }
        for (sel, node) in &keys {
            cache.get_or_insert(sel, *node, || true);
        }

        b.iter(|| {
            let mut hits = 0usize;
            for (sel, node) in &keys {
                if cache.get_or_insert(black_box(sel), black_box(*node), || unreachable!()) {
                    hits += 1;
                }
            }
            black_box(hits)
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// whitespace normalization
// ---------------------------------------------------------------------------

/// The pre-optimization `normalize_whitespace`: unconditional `replace` passes
/// plus a full rebuild, even when the text is already normalized.
fn normalize_whitespace_baseline(text: &str, white_space: WhiteSpace) -> String {
    let text = text.replace("\r\n", "\n");
    let text = text.replace(['\r', '\x0c'], "\n");

    let mut result = String::new();
    let mut prev_was_space = false;

    let is_space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c');

    for c in text.chars() {
        match white_space {
            WhiteSpace::Normal | WhiteSpace::Nowrap => {
                if is_space(c) {
                    if !prev_was_space {
                        result.push(' ');
                    }
                    prev_was_space = true;
                } else {
                    result.push(c);
                    prev_was_space = false;
                }
            }
            WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces => {
                result.push(c);
                prev_was_space = false;
            }
            WhiteSpace::PreLine => {
                if c == '\n' {
                    result.push('\n');
                    prev_was_space = false;
                } else if is_space(c) {
                    if !prev_was_space {
                        result.push(' ');
                    }
                    prev_was_space = true;
                } else {
                    result.push(c);
                    prev_was_space = false;
                }
            }
        }
    }

    if white_space == WhiteSpace::PreLine {
        while result.ends_with('\n') {
            result.pop();
        }
    }

    result
}

fn bench_normalize_whitespace(c: &mut Criterion) {
    let mut group = c.benchmark_group("normalize_whitespace");
    tune(&mut group);

    // Representative short text nodes: mostly already normalized, a few with
    // work to do.
    let samples: Vec<&str> = vec![
        "Rust",
        "programming language",
        "The quick brown fox jumps over the lazy dog.",
        "This line has  double  spaces  to collapse.",
        "trailing space ",
        "tab\tseparated\twords",
        "multi\nline\ntext",
        "CJK テキスト と English mixed",
    ];

    group.bench_function("optimized_normal", |b| {
        b.iter(|| {
            let mut sink = 0usize;
            for sample in &samples {
                sink +=
                    black_box(normalize_whitespace(black_box(sample), WhiteSpace::Normal)).len();
            }
            black_box(sink)
        });
    });

    group.bench_function("baseline_normal", |b| {
        b.iter(|| {
            let mut sink = 0usize;
            for sample in &samples {
                sink += black_box(normalize_whitespace_baseline(
                    black_box(sample),
                    WhiteSpace::Normal,
                ))
                .len();
            }
            black_box(sink)
        });
    });

    group.bench_function("optimized_borrow_ratio", |b| {
        b.iter(|| {
            let mut borrowed = 0usize;
            for sample in &samples {
                if matches!(
                    normalize_whitespace(black_box(sample), WhiteSpace::Normal),
                    Cow::Borrowed(_)
                ) {
                    borrowed += 1;
                }
            }
            black_box(borrowed)
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// HTTP response cache
// ---------------------------------------------------------------------------

fn bench_http_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("http_cache");
    tune(&mut group);

    let urls: Vec<String> = (0..512)
        .map(|i| format!("https://bench.test/page/{i}"))
        .collect();

    let response = |url: &str| Response {
        url: url.to_string(),
        status: StatusCode::from(hyper::StatusCode::OK),
        reason_phrase: "OK".to_string(),
        headers: vec![("cache-control".to_string(), "max-age=3600".to_string())],
        body: vec![0u8; 4096],
    };

    group.bench_function("hit", |b| {
        let cache = Cache::with_capacity(1024);
        for url in &urls {
            cache.set(url, &response(url));
        }

        let mut i = 0usize;
        b.iter(|| {
            let url = &urls[i % urls.len()];
            i += 1;
            black_box(cache.get(black_box(url)))
        });
    });

    group.bench_function("miss", |b| {
        let cache = Cache::with_capacity(1024);
        let mut i = 0usize;
        b.iter(|| {
            i += 1;
            black_box(cache.get(&format!("https://bench.test/missing/{i}")))
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_shaping,
    bench_text_shape_cache,
    bench_layout_cache,
    bench_selector_match_cache,
    bench_normalize_whitespace,
    bench_http_cache
);
criterion_main!(benches);
