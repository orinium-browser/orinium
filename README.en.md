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

<a href="./README.md" align="center">日本語</a>

> [!NOTE]
> This project is still under development and does not yet function as a browser.

> [!TIP]
> The code documentation is below. We always keep it up-to-date on the dev branch.
>
> https://orinium-browser.github.io/orinium/orinium_browser/

# Separating the Web from Proprietary Implementations

Orinium is a browser engine that aims to create an open execution environment where the Web is not dependent on the implementation or direction of any single organization, but remains under the control of users and developers.

Today, many browsers rely on Chromium. Chromium is an excellent open-source project, but the concentration of browser implementations around a single project creates risks for the diversity and independence of the Web.

When one implementation becomes the de facto standard, the interpretation of Web standards, the adoption of non-standard features, and the future direction of Web technologies can become heavily influenced by the decisions of one organization.

Orinium is not a project created to eliminate Google. Instead, it aims to prevent any single company or organization from becoming a controlling point of the Web, ensuring that the Web remains an open platform that anyone can participate in, improve, and build upon.

## Unique Extension Format

In the future, this browser engine will support extensions. The planned formats include:

- Orinium’s original format
- Firefox add-ons
- Chromium manifest v2 (partial support)

Supporting these formats helps maintain compatibility with other browsers, while unique features designed specifically for Orinium will provide a better user experience.

## Run

Clone the repository.

```bash
git clone https://github.com/orinium-browser/orinium.git
cd orinium
```

You can run Orinium using Cargo.

```bash
cargo run
```

### Development test harness

A development test harness is available at `examples/tests.rs`.

```bash
# Show available commands
cargo run --example tests help

# Parse DOM from a URL
cargo run --example tests parse_dom https://example.com

# Full rendering pipeline with a window
cargo run --example tests simple_render https://example.com
```

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md).

For the architecture, see [architecture.md](./docs/en/architecture.md).

Join the community to connect with other developers and stay up to date.
Join our Discord community: [here](https://discord.gg/tMGPgHFsxJ)

Other useful documentation for development can be found in [the directory](./docs/en).
