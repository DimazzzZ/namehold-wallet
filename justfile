# Just recipes for common development tasks.
# Install: `cargo install just`
# Usage: `just <recipe>`

# Run the full test suite
test:
    cd src-tauri && cargo test --lib

# Generate coverage report (requires nightly and cargo-llvm-cov).
# The --ignore-filename-regex excludes lib.rs and daemon/mod.rs (100% IO shell, no testable logic).
# The #[cfg_attr(coverage_nightly, coverage(off))] attributes on individual IO functions
# are inert on stable builds — this command must run on nightly to activate them.
coverage:
    cd src-tauri && cargo +nightly llvm-cov --lib --ignore-filename-regex '(src/lib\.rs|src/daemon/mod\.rs)'

# Build for release
build-release:
    cd src-tauri && cargo build --release

# Format code (stable toolchain, pre-dirty worktree OK)
fmt:
    cd src-tauri && cargo fmt --all

# Lint with clippy
lint:
    cd src-tauri && cargo clippy --lib -- -D warnings
