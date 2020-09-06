# cyclonedds-rs 

Rust bindings for cyclonedds https://github.com/eclipse-cyclonedds/cyclonedds.

## Dependant crates

1. cyclonedds-sys : sys crate for cyclonedds. Bindings generated using bindgen.
2. cycloneddscodegen : crate to generate bindings for an IDL using the Java IDLC generator

# Introduction

This crate allows you to use the cyclonedds library using(mostly) safe rust. Each IDL is maintained in
a separate crate. This allows us to use the power of cargo to version and maintain topic descriptors.

# Example - Roundrip from cyclonedds examples

Let us use the Roundtrip example from cyclonedds. 

## Part1 : Create the build infrastructure and IDL crate

    cargo new roundtrip-example
    cd roundtrip-example

Now create the library crate for the IDL.

    cargo generate --git https://github.com/sjames/dds_data_template.git 
    Project Name: roundtrip-data
    Creating project called `roundtrip-data`...
    Done! New project created /mnt/b46b1e02-5b78-4e18-a8b4-6dc68de87cda/work/roundtrip-example/roundtrip-data

A new library crate will be created in the workspace. Include this new crate into the Cargo.toml of the roundtrip-example crate.

    [dependencies]
    roundtrip-data = {path = "./roundtrip-data"}

The template provides an example IDL in the idl directory of the roundtrip-data crate. Let's change the contents to the IDL found in 
the roundtrip example. Also rename the file to RoundTrip.idl.

    module RoundTripModule
    {
    struct DataType
    {
        sequence<octet> payload;
    };
    #pragma keylist DataType
    };

Modify roundtrip-data/build.rs to reflect the new name.

    use cycloneddscodegen as codegen;
    fn main() {
        let idls = vec!["idl/Roundtrip.idl"];
        codegen::generate_and_compile_datatypes(idls);
    }

Make sure everything is ok by building the roundtrip-example project.

    cargo build










