<div align="center">

# 🦀 My Rust Apps

**Native Windows desktop apps written in Rust, built as fast and lightweight alternatives to everyday built-in software.**

[![Rust](https://img.shields.io/badge/Rust-Native-DEA584?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![egui](https://img.shields.io/badge/GUI-egui-4B8BBE?style=for-the-badge)](https://github.com/emilk/egui)
[![Slint](https://img.shields.io/badge/GUI-Slint-2379F4?style=for-the-badge)](https://slint.dev)
[![Platform](https://img.shields.io/badge/Platform-Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white)](#-requirements)

[Apps](#-apps) •
[Requirements](#-requirements) •
[Getting started](#-getting-started) •
[Repository structure](#-repository-structure) •
[Contributing](#-contributing)

</div>

---

## 📖 Overview

**My Rust Apps** is a collection of small desktop programs that rewrite common built-in desktop software as **native Rust applications**. They use no web view and no embedded browser, so they stay small, start quickly and use little memory.

Each app is an independent Cargo project in its own folder, using Rust-native GUI toolkits such as [egui](https://github.com/emilk/egui) and [Slint](https://slint.dev).

### Goals

- ⚡ **Fast**: native binaries with instant startup
- 🪶 **Light**: low memory use and small executables
- 🧩 **Focused**: each app does one job well
- 🦀 **Pure Rust**: Cargo-based, easy to build from source

---

## 📱 Apps

| App | Folder | Description |
| --- | ------ | ----------- |
| 📁 **Rust File** | [`rust-file`](./rust-file) | Native file explorer. |
| 🖼️ **Rust Photos** | [`rust-photos`](./rust-photos) | Native photo viewer. |

> 📝 The descriptions above are based on folder names only. Add the real feature list, screenshots and keyboard shortcuts for each app.

---

## 💻 Requirements

- **Windows 10 or 11** (64-bit)
- **Rust toolchain** (stable), installed with [rustup](https://rustup.rs)
- **Visual Studio C++ Build Tools** (MSVC) for the default Windows Rust target

---

## 🚀 Getting Started

### Clone the repository

```bash
git clone https://github.com/minhtrong67/my-rust-apps.git
cd my-rust-apps
```

### Run an app

```bash
cd rust-file       # or rust-photos
cargo run --release
```

### Build an executable

```bash
cd rust-file
cargo build --release
```

The binary is written to `target/release/` inside the app folder.

---

## 📁 Repository Structure

```text
my-rust-apps/
├── rust-file/       # Native file explorer
├── rust-photos/     # Native photo viewer
└── README.md
```

Each app folder is a standalone Cargo project with its own `Cargo.toml` and `src/`.

---

## 🧰 Tech Stack

| Area | Technology |
| ---- | ---------- |
| Language | Rust |
| Build system | Cargo |
| GUI | egui, Slint |
| Platform | Windows (native) |

---

## 🤝 Contributing

Issues and pull requests are welcome.

1. Fork the repository
2. Create a branch: `git checkout -b feat/my-change`
3. Run `cargo fmt` and `cargo clippy` before committing
4. Open a pull request describing what changed and how you tested it

---

## 📄 License

No license file is included yet. Add a `LICENSE` file (for example MIT or Apache 2.0) to define how others may use this code.

## 👤 Author

**minhtrong67** · [GitHub](https://github.com/minhtrong67)
