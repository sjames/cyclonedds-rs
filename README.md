# cyclonedds-rs 

Rust bindings for cyclonedds https://github.com/eclipse-cyclonedds/cyclonedds.

## Dependant crates

1. cyclonedds-sys : sys crate for cyclonedds. Bindings generated using bindgen.

This create no longer depends on a code generator. The Cyclone serialization
interface is used to implement the Rust interface. You can annotate a structure
with the new derive macro and start subscribing and publishing right from Rust.

# Introduction

This crate allows you to use the cyclonedds library using safe Rust. It uses the
cyclone serialization/deserialization interface for high performance and IDL free usage.


# Features

1. Qos
2. Reader and Writer
3. Listener with closure callbacks
4. Async reader 
5. multiple and nested keys
6. Uses

Complete example code implementing RoundTripPong is available at https://github.com/sjames/roundtrip-example . This works with
the CycloneDDS Ping example.

