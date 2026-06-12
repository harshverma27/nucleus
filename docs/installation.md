# Installation

## The `nucleus` CLI

### Prerequisites

- A **Rust** toolchain ≥ 1.85 (the MSRV). Install via [rustup](https://rustup.rs).
- Optional, only for `nucleus build` / `flash`: `arm-none-eabi-gcc`, `cmake`,
  `st-flash`, and a NUCLEO-F446RE board.

### From crates.io (once published)

```sh
cargo install nucleus-cli
```

This installs the `nucleus` binary into `~/.cargo/bin` (on your `PATH` if you
installed Rust via rustup).

> **Status:** publishing to crates.io is automated by the release workflow and
> activates on the first tagged release. Until then, install from source:

### From source

```sh
# From a clone
git clone https://github.com/harshverma27/nucleus
cd nucleus
cargo install --path crates/nucleus-cli --locked

# …or directly from GitHub
cargo install --git https://github.com/harshverma27/nucleus nucleus-cli --locked
```

### Prebuilt binaries

Each tagged release attaches prebuilt `nucleus` binaries for Linux, macOS, and
Windows (x86_64 + arm64), with SHA-256 checksums, on the
[Releases](https://github.com/harshverma27/nucleus/releases) page. Download,
verify, extract, and put `nucleus` on your `PATH`.

### Verify

```sh
nucleus --version
nucleus --help
```

## The VS Code extension

The extension is a thin client for the CLI (LSP + trace dashboard).

- **From the Marketplace** (once published): search for **Nucleus** in the
  Extensions view, or install the `.vsix` attached to a release with
  *Extensions: Install from VSIX…*.
- **From source:**
  ```sh
  cd extension
  npm install
  npm run build
  ```
  Then press **F5** in VS Code to launch an Extension Development Host.

The extension expects `nucleus` on your `PATH`; override with the
`nucleus.serverPath` setting if needed.
