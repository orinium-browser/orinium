<h1 align="center">Orinium Browser</h1>

<div align="center">
  <a href="./LICENSE" target="_blank"><img src="https://img.shields.io/github/license/orinium-browser/orinium" alt="Github license" /></a>
  <img alt="Static Badge" src="https://img.shields.io/badge/rustc-1.87%2B-blue">
  <a href="https://github.com/orinium-browser/orinium/actions" target="_blank"><img src="https://github.com/orinium-browser/orinium/actions/workflows/rust.yml/badge.svg" alt="Action Rust" /></a>
  <br>
  <a href="https://deps.rs/repo/github/orinium-browser/orinium" target="_blank"><img src="https://deps.rs/repo/github/orinium-browser/orinium/status.svg" alt="dependency status" /></a>
  <a href="https://deepwiki.com/orinium-browser/orinium" target="_blank"><img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki" /></a>
  <a href="https://discord.gg/2zYbEnMC5H" target="_blank"><img src="https://img.shields.io/badge/Discord-5865F2?style=flat&logo=discord&logoColor=white" alt="Discord server" /></a>
</div>

<a href="./README.en.md" align="center">English</a>

> [!NOTE]
> このプロジェクトは開発段階にあり、まだブラウザとして動作するわけではありません。

> [!TIP]
> 以下にコードのドキュメントがあります。常に、dev ブランチの最新を保っています。
>
> https://orinium-browser.github.io/orinium/orinium_browser/

## Googleに依存しない、独立したブラウザ

このブラウザエンジンのソースコードは、**Googleに依存しません**。Firefoxなどの一部のブラウザを除いて、世の中の多くのブラウザはGoogleのChromiumに依存しています。
このプロジェクトはChromiumに代る新しいブラウザエンジンを提供します。

## 拡張機能形式

将来的にこのブラウザエンジンは拡張機能をサポートします。現在サポート予定の形式は、

- Orinium 独自の形式
- Firefox addon
- Chromium manifest v2（部分的）

です。これらの機能のサポートは他のブラウザとの互換性を保つのに役立ち、またこのブラウザに適した独自の機能でより良いユーザーエクスペリエンスを提供できます。

## Run

リポジトリをクローンします。

```bash
git clone https://github.com/orinium-browser/orinium.git
cd orinium
```

Cargo を使って実行可能です。

```bash
cargo run
```

> [!NOTE]
> Ubuntu で以下のパッケージのインストールが必要になることが確認されています。
>
> ```bash
> sudo apt install pkg-config libasound2-dev
> ```

### 開発用テスト

開発用の test harness が `examples/tests.rs` にあります。

```bash
# コマンド一覧を表示
cargo run --example tests help

# URLを指定してDOMをパース
cargo run --example tests parse_dom https://example.com

# フルレンダリング（ウィンドウ表示）
cargo run --example tests simple_render https://example.com
```

## 貢献

[CONTRIBUTING.md](./CONTRIBUTING.md)を参照してください。

アーキテクチャは[architecture.md](./docs/ja/architecture.md)を参照してください。

コミュニティに参加すると、他の開発者と交流したり、最新情報を入手したりできます。
Discordコミュニティは[ここ](https://discord.gg/tMGPgHFsxJ)です！

その他の開発時に目を通しておくと便利なドキュメントは[ここ](./docs/ja)にあります。
