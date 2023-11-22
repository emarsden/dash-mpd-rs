# For use with the just command runner, https://just.systems/

export LLVM_PROFILE_FILE := 'coverage/cargo-test-%p-%m.profraw'

grcov:
  @echo 'Running tests for coverage with grcov'
  rm -rf ${CARGO_TARGET_DIR}/coverage
  CARGO_INCREMENTAL=0 RUSTFLAGS='-Cinstrument-coverage' cargo test
  grcov . --binary-path ${CARGO_TARGET_DIR}/debug/deps/ \
    -s . -t html \
    --branch \
    --ignore-not-existing --ignore '../*' --ignore "/*" \
    -o ${CARGO_TARGET_DIR}/coverage
  @echo grcov report in file://${CARGO_TARGET_DIR}/coverage/index.html


coverage-tarpaulin:
  @echo 'Running tests for coverage with tarpaulin'
  mkdir /tmp/tarpaulin
  cargo tarpaulin --engine llvm --line --out html --output-dir /tmp/tarpaulin


setup-coverage-tools:
  rustup component add llvm-tools-preview
  cargo install grcov
  cargo install cargo-tarpaulin
    

# Builds with the mold linker are faster (for Linux/AMD64)
moldy-build:
    mold -run cargo build


# Compiling the openssl crate on Android is complicated so we use the rustls-tls feature. Avoid some
# typing on tiny keyboards.
termux:
    cargo update
    cargo test --no-default-features --features fetch,rustls-tls,compression,scte35 -- --show-output


publish:
  cargo test
  cargo publish
