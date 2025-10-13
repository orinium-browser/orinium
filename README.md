# Orinium Browser
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/Orinas-github/Orinium-browser)

**🚧 _このプロジェクトは開発段階にあり、まだブラウザとして動作するわけではありません。_**

[English version → README.en.md](./README.en.md)

## Googleに依存しない、独立したブラウザ
このブラウザエンジンのソースコードは、**Googleに依存しません**。Firefoxなどの一部のブラウザを除いて、世の中の多くのブラウザはGoogleのChromiumに依存しています。
このプロジェクトはChromiumに代る新しいブラウザエンジンを提供します。

## 独自の拡張機能形式
将来的にこのブラウザエンジンは拡張機能をサポートします。現在サポート予定の形式は、
* Orinium 独自の形式
* Firefox addon
* Chromium manifest v2（部分的）

です。これらの機能のサポートは他のブラウザとの互換性を保つのに役立ち、またこのブラウザに適した独自の機能でより良いユーザーエクスペリエンスを提供できます。

---

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

---

## 貢献
[CONTRIBUTING.md](./CONTRIBUTING.md)を参照してください。

アーキテクチャは[architecture.md](./docs/architecture.md)を参照してください。

TODOは[tasks.md](./tasks.md)にあります。
