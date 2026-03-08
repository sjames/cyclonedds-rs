# cyclonedds-rs

Rust bindings for [Eclipse CycloneDDS](https://github.com/eclipse-cyclonedds/cyclonedds).
This crate no longer depends on a code generator. The Cyclone serialization
interface is used to implement the Rust interface. You can annotate a structure
with the new derive macro and start subscribing and publishing right from Rust.

A refactored fork of the original [cyclonedds-rs](https://github.com/sjames/cyclonedds-rs) and [cyclonedds-sys](https://github.com/sjames/cyclonedds-sys).

## Project Structure
This repository is organized as a Cargo Workspace:
- `cyclonedds`: High-level, safe Rust API.
- `cyclonedds-sys`: Raw FFI bindings (generated via bindgen).
- `cyclonedds-derive`: Procedural macros for DdsType.

## Acknowledgment
This project is based on the initial work by [sjames](https://github.com/sjames). We have refactored the architecture to improve maintainability and developer experience.

## License and Notice
This project is licensed under Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE) for details and attribution.

# Introduction

This crate allows you to use the cyclonedds library using safe Rust. It uses the
cyclone serialization/deserialization interface for high performance and IDL free usage.

# Features

1. Qos
2. Reader and Writer
3. Listener with closure callbacks
4. Async reader 
5. multiple and nested keys

# Special Instructions

The current release only supports the 0.10.X release branch. https://github.com/eclipse-cyclonedds/cyclonedds/tree/releases/0.10.x .

# Dependencies

* iceoryx https://github.com/eclipse-iceoryx/iceoryx version 2.0.2. (https://github.com/eclipse-iceoryx/iceoryx/commit/f756b7c99ddf714d05929374492b34c5c69355bb) Do not install any other version.
* cyclonedds 0.10.x branch (https://github.com/eclipse-cyclonedds/cyclonedds/commit/1be07de395e4ddf969db2b90328cdf4fb73e9a64) . Ensure that you build  and install Cyclone with SHM feature enabled. (cmake -DENABLE_SHM=1 ..)
* git
* libclang
* cmake
* make
* a C/C++ compiler for cmake to use

vendored 依存ライブラリのビルド手順は [vendor/README.md](vendor/README.md) を参照してください。
