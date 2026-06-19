# Changelog

## [Unreleased]

### Added
- **`--target` flag**: Document a crate for a target triple other than the host, so you can document platform-specific crates that don't build on your machine. For example, on macOS `cargo doc-md -p tokio-tun --target x86_64-unknown-linux-gnu` documents `tokio-tun`, which only builds on Linux and otherwise fails. Requires the target's standard library (`rustup target add <triple>`).

## [0.11.0] - 2025-11-10

### Added
- **Per-crate cleanup**: Each crate's output directory is now removed before regenerating documentation, eliminating stale module files from previous versions

### Fixed
- **JSON mode cleanup**: Now correctly uses crate name from JSON metadata instead of filename when cleaning up output directory

### Removed
- Legacy v0.7.x migration code (no longer needed)

## [0.10.3] - 2025-11-08

### Fixed
- **Output directory creation**: Now automatically creates parent directories for output path instead of failing when they don't exist

## [0.10.2] - 2025-11-07

### Fixed
- **Console output paths**: Fixed console output to show correct file paths with underscores for crates with hyphens in their names

## [0.10.1] - 2025-11-07

### Fixed
- **Master index links**: Fixed broken links for crates with hyphens in their names (index now correctly uses underscores to match directory structure)

## [0.10.0] - 2025-11-06

### Added
- **Extended item type support**: Added support for 5 additional Rust item types:
  - **Static variables**: Shows mutability and `unsafe` keyword when applicable
  - **Unions**: Full support with fields, generic parameters, and implementation blocks
  - **Declarative macros** (`macro_rules!`): Displays complete macro definition
  - **Procedural macros**: Shows invocation syntax with kind-specific examples (derive, attribute, function-like)
  - **Trait aliases**: Displays aliased traits with generic parameters and bounds

### Improved
- **Constant documentation**: Now displays type information (e.g., `*Constant*: \`usize\``)
- **Macro labeling**: Changed from ambiguous "Macro" to specific "Declarative Macro"
- **Summary statistics**: Fixed pluralization in module summaries (e.g., "1 struct" instead of "1 structs")
- **Union documentation**: Includes methods and trait implementations like structs and enums

## [0.9.0] - 2025-11-06

### Added
- **Workspace support**: `--workspace` flag to document all workspace members and their dependencies
- **Package selection**: `-p/--package` flag to document specific packages (can be repeated)
- **Dependency control**: `--no-deps` flag to exclude dependencies (matches `cargo doc --no-deps`)
- **Transitive dependencies**: Now documents all transitive dependencies, not just direct ones
- **Platform filtering**: Automatically filters platform-specific dependencies using `cargo metadata --filter-platform`

### Fixed
- **`--include-private` flag now works**: Properly passes `--document-private-items` to rustdoc
- **Renamed lib targets**: Correctly handles crates where `[lib] name` differs from package name
- **Binary-only crates**: Now properly skipped instead of marked as successful
- **Target directory detection**: Uses `metadata["target_directory"]` instead of hardcoded "target/doc"
- **Output directory creation**: Ensures output directory exists before writing master index
- **Error handling**: Replaced unsafe unwraps with proper error handling using let-else pattern
- **Workspace member deduplication**: Prevents workspace crates from being scheduled twice as dependencies
- **CLI flag conflicts**: Added proper `conflicts_with` attributes in Clap for mutually exclusive flags

### Changed
- CLI now uses `-p` flag instead of positional arguments for package names
- README updated to reflect actual CLI interface (removed non-existent `--all-deps` flag)
- Master index generation now handles edge cases (empty workspaces, binary-only crates)
- Improved error messages for virtual workspaces and missing packages

## [0.8.1] - 2025-10-21

### Fixed
- Removed `--no-deps` flag from cargo metadata call to fix "Missing 'resolve.root'" error

## [0.8.0] - 2025-10-21

### Changed (Breaking)
- **Flattened directory structure**: Dependencies now at `target/doc-md/crate/` instead of `target/doc-md/deps/crate/`
- **CLI redesign**: Positional args now accept crate names instead of JSON paths
  - Old: `cargo doc-md --deps tokio,serde`
  - New: `cargo doc-md tokio serde`
- **JSON conversion**: Now requires explicit `--json` flag
  - Old: `cargo doc-md file.json`
  - New: `cargo doc-md --json file.json`

### Added
- Automatic migration from old `deps/` structure on first run
- Nightly toolchain validation with helpful error messages
- Flag conflict validation (e.g., `--json` + crate names)
- Better error messages with actionable suggestions
- CLI integration tests

### Fixed
- Root package lookup now uses `resolve.root` (correct for workspaces)
- Stderr shown on build failures (was suppressed)
- Master index now generated in all code paths

### Migration from 0.7.x
Tool auto-migrates on first run. Update scripts:
- Replace `--deps crate1,crate2` with `crate1 crate2`
- Replace `cargo doc-md file.json` with `cargo doc-md --json file.json`
- Update file paths: `deps/crate/` → `crate/`
