#!/bin/bash

unset CC_aarch64_linux_android
unset CXX_aarch64_linux_android
cargo build --release --manifest-path $ROOT/rust/carbine_fedimint/Cargo.toml --target-dir $ROOT/rust/carbine_fedimint/target
