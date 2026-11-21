//! Tokenizer + editref benchmarks.
//!
//! Run with `cargo bench --bench tokenizer_editref_bench`.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};

use editref::EditRefSource;
use orinium_browser::engine::css::tokenizer::Tokenizer as CssTokenizer;
use orinium_browser::engine::html::tokenizer::Tokenizer as HtmlTokenizer;

// ---------------------------------------------------------------------------
// Sample data
// ---------------------------------------------------------------------------

const CSS_SAMPLE: &str = r#"
/* Base styles */
html {
  font-family: sans-serif;
  line-height: 1.5;
  color: #333;
}

body {
  margin: 0;
  padding: 0;
  background-color: #fff;
}

.header {
  background: linear-gradient(90deg, #ff6b6b, #4ecdc4);
  padding: 1rem;
  text-align: center;
}

.nav a {
  color: #2c3e50;
  text-decoration: none;
  margin: 0 0.5rem;
}

.nav a:hover {
  color: #e74c3c;
}

.card {
  border: 1px solid #ddd;
  border-radius: 8px;
  padding: 1.5rem;
  box-shadow: 0 2px 4px rgba(0, 0, 0, 0.1);
}

@media (max-width: 768px) {
  .card {
    padding: 1rem;
  }
}
"#;

const HTML_SAMPLE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Benchmark Page</title>
  <link rel="stylesheet" href="style.css">
  <script src="app.js"></script>
</head>
<body>
  <header class="site-header">
    <nav>
      <ul>
        <li><a href="/">Home</a></li>
        <li><a href="/about">About</a></li>
        <li><a href="/contact">Contact</a></li>
      </ul>
    </nav>
  </header>
  <main>
    <article>
      <h1>Article Title</h1>
      <p>This is a paragraph with <strong>bold</strong> and <em>italic</em> text.</p>
      <p>Another paragraph with <a href="/link">a link</a>.</p>
      <img src="image.png" alt="An image" width="300" height="200">
    </article>
    <aside>
      <h2>Sidebar</h2>
      <ul>
        <li>Item 1</li>
        <li>Item 2</li>
        <li>Item 3</li>
      </ul>
    </aside>
  </main>
  <footer>
    <p>&copy; 2024 Benchmark</p>
  </footer>
</body>
</html>"#;

// ---------------------------------------------------------------------------
// editref: sequential get() access
// ---------------------------------------------------------------------------

fn bench_editref_get_sequential(c: &mut Criterion) {
    let mut group = c.benchmark_group("editref::get_sequential");
    group.measurement_time(Duration::from_secs(3));
    group.warm_up_time(Duration::from_secs(1));

    let source = EditRefSource::from_slice(&CSS_SAMPLE.chars().collect::<Vec<_>>());
    let view = source.slice(..);

    group.bench_function("get_each_char", |b| {
        b.iter(|| {
            let mut sum = 0usize;
            for i in 0..view.len() {
                sum += black_box(view.get(i)).is_some() as usize;
            }
            black_box(sum)
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// editref: get_range vs as_slice_range vs get-loop for lexeme extraction
// ---------------------------------------------------------------------------

fn bench_editref_lexeme_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("editref::lexeme_extraction");
    group.measurement_time(Duration::from_secs(3));
    group.warm_up_time(Duration::from_secs(1));

    let source = EditRefSource::from_slice(&CSS_SAMPLE.chars().collect::<Vec<_>>());
    let view = source.slice(..);

    // Pretend we want to extract a lexeme of ~50 chars starting at position 100
    const START: usize = 100;
    const LEN: usize = 50;

    group.bench_function("as_slice_range_zero_copy", |b| {
        b.iter(|| {
            let slice = black_box(view.as_slice_range(START, START + LEN));
            black_box(slice.map(|s| s.len()).unwrap_or(0))
        });
    });

    group.bench_function("get_range_collect", |b| {
        b.iter(|| {
            let vec = black_box(view.get_range(START, START + LEN));
            black_box(vec.map(|v| v.len()).unwrap_or(0))
        });
    });

    group.bench_function("get_loop_collect", |b| {
        b.iter(|| {
            let s: String = (START..START + LEN)
                .filter_map(|i| view.get(i))
                .collect();
            black_box(s.len())
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// editref: replace + re-access
// ---------------------------------------------------------------------------

fn bench_editref_replace(c: &mut Criterion) {
    let mut group = c.benchmark_group("editref::replace");
    group.measurement_time(Duration::from_secs(3));
    group.warm_up_time(Duration::from_secs(1));

    let source = EditRefSource::from_slice(&CSS_SAMPLE.chars().collect::<Vec<_>>());

    group.bench_function("replace_middle_and_reaccess", |b| {
        b.iter(|| {
            let mut view = source.slice(..);
            view.replace(50..60, "EDITED".chars());
            let sum: usize = (0..view.len())
                .filter_map(|i| view.get(i))
                .map(|c| *c as usize)
                .sum();
            black_box(sum)
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// CSS tokenizer
// ---------------------------------------------------------------------------

fn bench_css_tokenizer(c: &mut Criterion) {
    let mut group = c.benchmark_group("css::tokenizer");
    group.measurement_time(Duration::from_secs(3));
    group.warm_up_time(Duration::from_secs(1));

    // Repeat the sample to make it larger
    let css = CSS_SAMPLE.repeat(20);

    group.bench_function("tokenize", |b| {
        b.iter(|| {
            let mut tok = CssTokenizer::new(&css);
            let mut count = 0usize;
            loop {
                let token = tok.next_token();
                count += 1;
                if matches!(token, orinium_browser::engine::css::tokenizer::Token::EOF) {
                    break;
                }
            }
            black_box(count)
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// HTML tokenizer
// ---------------------------------------------------------------------------

fn bench_html_tokenizer(c: &mut Criterion) {
    let mut group = c.benchmark_group("html::tokenizer");
    group.measurement_time(Duration::from_secs(3));
    group.warm_up_time(Duration::from_secs(1));

    let html = HTML_SAMPLE.repeat(10);

    group.bench_function("tokenize", |b| {
        b.iter(|| {
            let mut tok = HtmlTokenizer::new(&html);
            let mut count = 0usize;
            loop {
                let token = tok.next_token();
                count += 1;
                if token.is_none() {
                    break;
                }
            }
            black_box(count)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_editref_get_sequential,
    bench_editref_lexeme_extraction,
    bench_editref_replace,
    bench_css_tokenizer,
    bench_html_tokenizer
);
criterion_main!(benches);
