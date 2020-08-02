use cycloneddscodegen as codegen;

fn main() {

  let idls = vec!["idl/HelloWorldData.idl"];
  codegen::generate_and_compile_datatypes(idls);

}

