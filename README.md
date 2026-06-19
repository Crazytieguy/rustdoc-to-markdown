# cargo-doc-md

> **🤖 AI-Generated**: Created by Claude (Anthropic AI)

A Cargo subcommand that generates markdown documentation for Rust crates and their dependencies.

## Installation

**Requires Rust nightly** (uses unstable rustdoc features):
```bash
rustup install nightly
cargo install cargo-doc-md
```

## Usage

```bash
# Document current crate + all dependencies
cargo doc-md

# Document specific packages
cargo doc-md -p tokio -p serde

# Document all workspace members
cargo doc-md --workspace

# Custom output directory
cargo doc-md -o docs/

# Document a crate for another target (e.g. a Linux-only crate from macOS)
cargo doc-md -p tokio-tun --target x86_64-unknown-linux-gnu
```

Run `cargo doc-md --help` for all options.

### Output Structure

```
target/doc-md/
  index.md                    # Master index
  your_crate/
    index.md                  # Crate overview
    module1.md                # One file per module
    sub/
      nested_module.md
  tokio/
    index.md
    ...
```

## Features

- Multi-file output with one markdown file per module
- Master index listing all documented crates
- Breadcrumb navigation showing module hierarchy
- Automatic dependency discovery and documentation
- Handles multiple versions of the same dependency

## License

MIT or Apache-2.0
