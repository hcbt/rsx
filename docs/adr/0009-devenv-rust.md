# devenv is the toolchain

Rust is built and tested inside devenv with a pinned toolchain. `devenv shell -- cargo test` and friends. Host `rustc` / rustup are not the project toolchain.
