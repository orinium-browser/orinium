<h1 align="center">Orinium Browser</h1>

<div align="center">
  <a href="https://deepwiki.com/orinium-browser/orinium" target="_blank"><img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki" /></a>
  <a href="./LICENSE" target="_blank"><img src="https://img.shields.io/github/license/orinium-browser/orinium" alt="Github license" /></a>
  <a href="https://discord.gg/2zYbEnMC5H" target="_blank"><img src="https://img.shields.io/badge/Discord-5865F2?style=flat&logo=discord&logoColor=white" alt="Discord server" /></a>
  

  <a href="https://github.com/orinium-browser/orinium/actions" target="_blank"><img src="https://github.com/orinium-browser/orinium/actions/workflows/rust.yml/badge.svg" alt="Action Rust" /></a>
  <a href="https://deps.rs/repo/github/orinium-browser/orinium" target="_blank"><img src="https://deps.rs/repo/github/orinium-browser/orinium/status.svg" alt="dependency status" /></a>
</div>

<a href="./README.md" align="center">日本語</a>

> [!NOTE]
> This project is still under development and does not yet function as a browser.

## An Independent Browser Free from Google
The source code of this browser engine **does not depend on Google**.  
Except for a few browsers like Firefox, most browsers in the world rely on Google’s Chromium.  
This project aims to provide a new browser engine as an alternative to Chromium.

## Unique Extension Format
In the future, this browser engine will support extensions. The planned formats include:
* Orinium’s original format  
* Firefox add-ons  
* Chromium manifest v2 (partial support)

Supporting these formats helps maintain compatibility with other browsers, while unique features designed specifically for Orinium will provide a better user experience.

## Run
Clone the repository.

```bash
git clone https://github.com/orinium-browser/orinium.git
cd orinium
```
> [!NOTE]
> Orinium's MSRV (Minimum Supported Rust Version) is 1.88.0.
> If you are using an earlier version, please upgrade to Rust 1.88.0 or later.
> 
> If you are using rustup, you can set the required version with the following commands:
> 
> ```bash
> rustup toolchain install 1.88.0
> rustup override set 1.88.0
> ```
You can run Orinium using Cargo.

```bash
cargo run
```

## Contributing
See [CONTRIBUTING.md](./CONTRIBUTING.md).

For the architecture, see [architecture.md](./docs/en/architecture.md).

Join the community to connect with other developers and stay up to date.
Join our Discord community: [here](https://discord.gg/tMGPgHFsxJ)

Other useful documentation for development can be found in [the directory](./docs/en).
And, most documents are classified based on their lang.
