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
| コマンド              | 内容                         |
|-------------------|----------------------------|
| `help`            | コマンド一覧を表示                  |
| `fetch_url <URL>` | 指定URLを取得し、レスポンスを表示         |
| `parse_dom <URL>` | URLからHTMLを取得し、DOMツリーを構築・出力 |

#### 例
```bash
# ネットワーク通信テスト
cargo run --example tests fetch_url https://example.com

# DOMパーステスト
cargo run --example tests parse_dom https://example.com
```

この example は、`#[test]` では実行しづらい非同期処理やGUI処理を手軽に確認するためのものです。
