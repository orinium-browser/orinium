# コミットメッセージガイドライン

このドキュメントでは、Orinium Browser のコミットメッセージの書き方を説明します。

## 基本形式

コミットメッセージは、次の形式で記述します。

```text
<type>(<scope>)!: <summary>
```

必要に応じて以下の body をつけます。

```text
<body>

BREAKING CHANGE:
<description>
```

### 例

```text
feat: add CSS variables
fix(layout): prevent integer overflow
perf: reduce memory usage
```

## type

`type` は変更の種類を表します。

| type       | 内容                                   |
| ---------- | -------------------------------------- |
| `feat`     | 機能の追加・拡張                       |
| `fix`      | 不具合の修正                           |
| `refactor` | 動作を変えないコードの改善・整理       |
| `perf`     | パフォーマンスやリソース使用量の改善   |
| `docs`     | ドキュメントの追加・修正               |
| `test`     | テストの追加・修正                     |
| `build`    | ビルドシステムや依存関係の変更         |
| `ci`       | CI の設定変更                          |
| `style`    | コードスタイルやフォーマットのみの変更 |
| `chore`    | その他の保守・管理作業                 |
| `revert`   | コミットの取り消し                     |

## scope

`scope` は変更した機能やモジュールを表します。

必要な場合のみ指定してください。

例:

- `layout`
- `css`
- `dom`
- `parser`
- `network`

## Summary

Summary は、変更内容を一行で簡潔に表します。

- 動詞で書き始めます。
- 何を変更したかが分かるように記述します。
- 詳しい説明が必要な場合は Body に記述します。

### 用語の例

Summary では、変更内容を表す動詞を使用します。

| 意味   | 例         |
| ------ | ---------- |
| 追加   | `add`      |
| 対応   | `support`  |
| 修正   | `fix`      |
| 防止   | `prevent`  |
| 改善   | `improve`  |
| 単純化 | `simplify` |
| 最適化 | `optimize` |
| 削除   | `remove`   |
| 置換   | `replace`  |
| 更新   | `update`   |

## Body

Summary だけでは説明できない内容がある場合に記述します。

変更理由や実装方法などを必要に応じて記述してください。

## BREAKING CHANGE

互換性のない変更について、影響や補足が必要な場合に記述します。

## 記述ルール

- 1 つのコミットでは、1 つの論理的な変更のみを行います。
- Summary は変更内容を簡潔に記述します。
- Body は必要な場合のみ記述します。
- 同じ種類の変更では、できるだけ同じ表現を使用します。
- 一貫性を重視してください。
