# The Android build environment: Rust, the Android SDK and NDK, and cargo-apk.
# Google ships the NDK for x86_64 Linux hosts only, so build.sh builds this image for linux/amd64
# (emulated on Apple silicon).
#
# Building this image accepts the Android SDK license terms (sdkmanager --licenses).
FROM rust:1.99-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends openjdk-17-jdk-headless unzip \
    && rm -rf /var/lib/apt/lists/*

ENV ANDROID_HOME=/opt/android-sdk
ARG CMDLINE_TOOLS=commandlinetools-linux-11076708_latest.zip
RUN curl -fsSL "https://dl.google.com/android/repository/$CMDLINE_TOOLS" -o /tmp/tools.zip \
    && mkdir -p "$ANDROID_HOME/cmdline-tools" \
    && unzip -q /tmp/tools.zip -d "$ANDROID_HOME/cmdline-tools" \
    && mv "$ANDROID_HOME/cmdline-tools/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest" \
    && rm /tmp/tools.zip

ARG NDK=27.2.12479018
RUN yes | "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --licenses > /dev/null \
    && "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --install \
        "platforms;android-34" "build-tools;34.0.0" "ndk;$NDK" > /dev/null
ENV ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/$NDK

RUN rustup target add aarch64-linux-android \
    && cargo install --locked cargo-apk

WORKDIR /src
