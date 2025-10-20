# Developer Docs
開発者が開発する際に使用できるドキュメントです

## 🧪 開発用テスト（Examples）
`examples/tests.rs` には、Orinium Browser の主要コンポーネントを個別に動作確認できる開発用テストが含まれています。  
GUI・ネットワーク・HTMLパーサなどを統合的にチェックすることができます。

### 実行方法
```bash
cargo run --example tests help
```

### 使用例
| コマンド           | 内容                       |
| ----------------- | -------------------------- |
| `help`            | コマンド一覧を表示           |
| `create_window`   | ウィンドウを作成して表示      |
| `fetch_url <URL>` | 指定URLを取得し、レスポンスを表示 |
| `parse_dom <URL>` | URLからHTMLを取得し、DOMツリーを構築・出力 |

#### 例
```bash
# ウィンドウ作成テスト
cargo run --example tests create_window

# ネットワーク通信テスト
cargo run --example tests fetch_url https://example.com

# DOMパーステスト
cargo run --example tests parse_dom https://example.com
```

この example は、`#[test]` では実行しづらい非同期処理やGUI処理を手軽に確認するためのものです。
