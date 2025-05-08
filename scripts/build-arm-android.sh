#!/bin/bash

cd $ROOT/rust/carbine_fedimint
export CC_aarch64_linux_android=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android21-clang
export CXX_aarch64_linux_android=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android21-clang++
cargo ndk -t arm64-v8a -o $ROOT/android/app/src/main/jniLibs build --release --target aarch64-linux-android
cp $ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so $ROOT/android/app/src/main/jniLibs/arm64-v8a/
