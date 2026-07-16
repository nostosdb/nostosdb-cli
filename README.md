# nostos-cli

Source-available command-line consumer of the NostosDB engine, licensed under SSPL-1.0.

Stage 0 supplies a compiling binary with `--help` and `--version` only. Query commands, synchronization, output formats, exit-code policy, and the REPL are deferred to Stage 7.

The local development manifest depends on sibling `nostos-engine` by path and compatible version. It does not access Core storage internals.

## Verify

```bash
cargo metadata --no-deps
cargo fmt --all --check
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run -- --help
cargo run -- --version
```

## License

Source-available under SSPL-1.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
