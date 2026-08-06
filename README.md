# cyclonedds-rs 

Rust bindings for cyclonedds https://github.com/eclipse-cyclonedds/cyclonedds.
This crate no longer depends on a code generator. The Cyclone serialization
interface is used to implement the Rust interface. You can annotate a structure
with the new derive macro and start subscribing and publishing right from Rust.

# Introduction

This crate allows you to use the cyclonedds library using safe Rust. It uses the
cyclone serialization/deserialization interface for high performance and IDL free usage.

# Features

1. Qos, including loading QoS profiles from XML at runtime (`DdsQosProvider`)
2. Reader and Writer, including zero-copy loans (`loan`/`loan_of_size`) and
   dispose/unregister_instance/writedispose
3. Listener with closure callbacks
4. Async reader
5. multiple and nested keys
6. Shared memory support using iceoryx
7. Dynamic Types: define a topic type's fields at runtime instead of generating a Rust
   struct at compile time via `cdds_derive` (see `docs/design/dynamic-types.md` and
   `examples/dynamic_types_demo.rs`). Currently limited to flat, `FINAL`-extensibility
   structs.

# Examples

1. https://github.com/sjames/demo-vehicle-speed-subscriber  (Vehicle speed subscriber with async reader)
2. https://github.com/sjames/demo-vehicle-speed-publisher (Vehicle speed publisher)
3. `examples/dynamic_types_demo.rs` - guided tour of the Dynamic Type API in a single process
4. `examples/dynamic_cross_writer.rs` / `examples/dynamic_cross_reader.rs` - the same Dynamic
   Type sample split across two real OS processes talking over the network

# Special Instructions

This release targets CycloneDDS 11. https://github.com/eclipse-cyclonedds/cyclonedds/releases/tag/11.0.1 .
Install this before building this crate or the examples.

# Dependencies

* iceoryx (iceoryx_hoofs + iceoryx_posh), only required if building with the `shm` feature (enabled by default). As of CycloneDDS 11, Iceoryx is built as a separate PSMX plugin (`libpsmx_iox`) discovered via CMake's `find_package`, rather than linked directly, so recent iceoryx releases should work - it is no longer pinned to a specific commit.
* cyclonedds 11.0.1 (https://github.com/eclipse-cyclonedds/cyclonedds/releases/tag/11.0.1). Ensure that you build and install Cyclone with SHM feature enabled. (cmake -DENABLE_SHM=1 ..)
* git
* libclang
* cmake
* make
* a C/C++ compiler for cmake to use
