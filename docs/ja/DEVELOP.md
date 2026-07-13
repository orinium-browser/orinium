# Developer Docs
開発者が開発する際に使用できるドキュメントです

## 🧪 開発用テスト（Examples）
`examples/tests.rs` には、Orinium Browser の主要コンポーネントを個別に動作確認できる開発用テストが含まれています。  
GUI・ネットワーク・HTMLパーサなどを統合的にチェックすることができます。
> [!WARNING]
> [使用例](#使用例), [例](#例) などは古くなり、コマンドが削除されたりしている可能性があります。使用する前に
> ```bash
> cargo run --example tests help
> ```
> でコマンドを確認してください

### 実行方法
```bash
cargo run --example tests help
```

### 使用例
| コマンド                  | 内容                               |
|-----------------------|----------------------------------|
| `help`                | コマンド一覧を表示                        |
| `fetch_url <URL>`     | 指定URLを取得し、ステータスとヘッダー、本文を表示      |
| `parse_dom <URL>`     | URLからHTMLを取得し、DOMツリーを構築・出力       |
| `parse_cssom <URL>`   | URLからCSSを取得し、CSSOMツリーを構築・出力      |
| `send_request <URL>`  | リダイレクトなしでHTTPリクエストを送信             |
| `dump_infonode <URL>` | HTML/CSSを取得し、InfoNode（レンダリング情報）を表示 |
| `dump_layoutnode <URL>` | HTML/CSSを取得し、レイアウト計算結果を表示         |
| `dump_draw_command <URL>` | レイアウトからDrawCommandを生成して表示          |
| `simple_render <URL>` | フルパイプライン（取得→レイアウト→描画）を実行         |

#### 例
```bash
# ネットワーク通信テスト
cargo run --example tests fetch_url https://example.com

# DOMパーステスト
cargo run --example tests parse_dom https://example.com

# CSSOMパーステスト
cargo run --example tests parse_cssom https://example.com

# レイアウト情報表示
cargo run --example tests dump_layoutnode https://example.com

# フルレンダリング
cargo run --example tests simple_render https://example.com
```

この example は、`#[test]` では実行しづらい非同期処理やGUI処理を手軽に確認するためのものです。
