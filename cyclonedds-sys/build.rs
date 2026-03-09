// Sojan James
// build.rs for cyclonedds-sys

/*
    Copyright 2020 Sojan James

    Licensed under the Apache License, Version 2.0 (the "License");
    you may not use this file except in compliance with the License.
    You may obtain a copy of the License at

        http://www.apache.org/licenses/LICENSE-2.0

    Unless required by applicable law or agreed to in writing, software
    distributed under the License is distributed on an "AS IS" BASIS,
    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    See the License for the specific language governing permissions and
    limitations under the License.
*/

use std::env;
use std::process::Command;

// Search paths for CycloneDDS, Iceoryx installation
const SEARCH_TARGETS: &[&str] = &["/usr", "/usr/local"];

fn main() {
    // Don't try to re-generate in docs.rs' build runner.
    if let Ok(val) = env::var("DOCS_RS") {
        if val == "1" {
            return;
        }
    }
    build::main();
}

macro_rules! ok(($expression:expr) => ($expression.unwrap()));
macro_rules! log {
    ($fmt:expr) => (println!(concat!("cyclonedds-sys/build.rs:{}: ", $fmt), line!()));
    ($fmt:expr, $($arg:tt)*) => (println!(concat!("cyclonedds-sys/build.rs:{}: ", $fmt),
    line!(), $($arg)*));
}

fn run<F>(name: &str, mut configure: F)
where
    F: FnMut(&mut Command) -> &mut Command,
{
    let mut command = Command::new(name);
    let configured = configure(&mut command);
    log!("Executing {:?}", configured);
    if !ok!(configured.status()).success() {
        panic!("failed to execute {:?}", configured);
    }
    log!("Command {:?} finished successfully", configured);
}

mod build {

    extern crate bindgen;

    use super::*;
    use glob::glob;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;

    static ENV_PREFIX: &str = "CYCLONEDDS";
    static LINKLIB: &str = "ddsc";
    static GIT_COMMIT: &str = "76360fb73907ce3dba397e89090a7a4ecf4f1246";

    #[allow(clippy::enum_variant_names)]
    pub enum HeaderLocation {
        FromCMakeEnvironment(std::vec::Vec<String>, String),
        FromEnvironment(std::vec::Vec<String>),
        FromLocalBuild(std::vec::Vec<String>),
    }

    impl HeaderLocation {
        #[cfg(feature = "shm")]
        fn add_paths(&mut self, mut path: Vec<String>) {
            match self {
                HeaderLocation::FromCMakeEnvironment(paths, _) => paths.append(&mut path),
                HeaderLocation::FromEnvironment(paths) => paths.append(&mut path),
                HeaderLocation::FromLocalBuild(paths) => paths.append(&mut path),
            }
        }

        #[cfg(feature = "shm")]
        fn get_paths(&self) -> Vec<String> {
            match self {
                HeaderLocation::FromCMakeEnvironment(paths, _)
                | HeaderLocation::FromEnvironment(paths)
                | HeaderLocation::FromLocalBuild(paths) => paths.clone(),
            }
        }
    }

    /// download cyclone dds from github
    fn download() {
        // get head of master for now. We can change to a specific version when
        // needed

        let outdir = env::var("OUT_DIR").expect("OUT_DIR is not set");
        let srcpath = format!("{}/cyclonedds", &outdir);
        let cyclonedds_src_path = Path::new(srcpath.as_str());

        if !cyclonedds_src_path.exists() {
            log!("Cloning cyclonedds from github");
            run("git", |command| {
                command
                    .arg("clone")
                    .arg("https://github.com/eclipse-cyclonedds/cyclonedds.git")
                    .current_dir(env::var("OUT_DIR").expect("OUT_DIR is not set").as_str())
            });
        }
        log!("running git checkout to get the right version of cyclonedds");
        run("git", |command| {
            command
                .arg("checkout")
                .arg(GIT_COMMIT)
                .current_dir(cyclonedds_src_path.to_str().unwrap())
        });
    }

    fn configure_and_build() {
        let outdir = env::var("OUT_DIR").expect("OUT_DIR is not set");
        let srcpath = format!("{}/cyclonedds", &outdir);
        let cyclonedds_src_path = Path::new(srcpath.as_str());

        run("mkdir", |command| {
            command
                .arg("-p")
                .arg("build")
                .current_dir(cyclonedds_src_path.to_str().unwrap())
        });

        run("cmake", |command| {
            command
                .env("CFLAGS", "-w")
                .arg("-DWERROR=OFF")
                .arg("-DBUILD_IDLC=OFF")
                .arg("-DBUILD_DDSPERF=OFF")
                .arg("-DBUILD_TESTING=OFF")
                .arg("-DBUILD_DDSPERF=OFF")
                .arg("-DENABLE_TYPE_DISCOVERY=YES")
                .arg("-DENABLE_TOPIC_DISCOVERY=YES")
                .arg(format!("-DCMAKE_INSTALL_PREFIX={}/install", outdir))
                .arg("..")
                .current_dir(format!("{}/build", cyclonedds_src_path.to_str().unwrap()));

            #[cfg(feature = "shm")]
            command.arg("-DENABLE_SHM=YES");

            #[cfg(not(feature = "shm"))]
            command.arg("-DENABLE_SHM=NO");

            command
        });

        run("make", |command| {
            command
                .env("MAKEFLAGS", env::var("CARGO_MAKEFLAGS").unwrap())
                .current_dir(format!("{}/build", cyclonedds_src_path.to_str().unwrap()))
        });

        run("make", |command| {
            command
                .arg("install")
                .current_dir(format!("{}/build", cyclonedds_src_path.to_str().unwrap()))
        });

        println!("cargo:rustc-link-search=native={}", outdir);
        println!("cargo:rustc-link-lib=dylib=ddsc");
        //cargo:rustc-link-lib=LIB
    }

    // Iceoryxを順番に探索
    // 1. インストール済みのiceoryxのヘッダーファイルを探す
    // 2. ローカルビルドのヘッダーファイルを探す
    #[cfg(feature = "shm")]
    fn find_iceoryx(iceoryx_version: &str) -> Option<HeaderLocation> {
        for target in SEARCH_TARGETS {
            let iceoryx_header_path = format!(
                "{}/include/iceoryx/{}/iceoryx_binding_c/api.h",
                target, iceoryx_version
            );
            let header = PathBuf::from(&iceoryx_header_path);
            if header.exists() {
                let iceoryx_include_path =
                    header.parent().unwrap().parent().unwrap().to_str().unwrap();
                let paths = vec![iceoryx_include_path.into()];
                return Some(HeaderLocation::FromEnvironment(paths));
            }
        }
        println!("cargo:warning=Iceoryx headers not found");
        None
    }

    fn find_cyclonedds() -> Option<HeaderLocation> {
        // The library name does not change. Print that out right away
        println!("cargo:rustc-link-lib={}", LINKLIB);

        let outdir = env::var("OUT_DIR").expect("OUT_DIR is not set");

        //first priority is environment variable.
        if let Ok(dir) = env::var(format!("{}_LIB_DIR", ENV_PREFIX)) {
            println!("cargo:rustc-link-search={}", dir);

            // Now find the include path
            if let Ok(dir) = env::var(format!("{}_INCLUDE_DIR", ENV_PREFIX)) {
                let path = format!("{}/dds/dds.h", &dir);
                let path = Path::new(&path);
                if path.exists() {
                    println!("Found {}", &path.to_str().unwrap());
                    let paths = vec![dir];
                    Some(HeaderLocation::FromEnvironment(paths))
                } else {
                    println!("Cannot find dds/dds.h");
                    None
                }
            } else {
                println!("LIB_DIR set but INCLUDE_DIR is unset");
                None
            }
        }
        // now check if building using CMAKE. CycloneDDS has a cmake
        // build environment. When building within CMake, the cyclonedds need not
        // be "installed", so multiple include paths are required.
        else if let Ok(dir) = env::var("CMAKE_BINARY_DIR") {
            let cmake_bin_dir = &dir;
            let lib_dir = Path::new(&dir).join("lib");
            println!("cargo:rustc-link-search={:}", lib_dir.display());

            if let Ok(dir) = env::var("CMAKE_SOURCE_DIR") {
                println!(
                    "CMAKE_SOURCE_DIR is set to {}, searching for include path",
                    &dir
                );
                let cmake_src_dir = Path::new(&dir);
                let glob_pattern = format!("{}/**/dds/dds.h", cmake_src_dir.display());
                println!("Glob pattern: {}", &glob_pattern);
                let mut paths = std::vec::Vec::new();
                for entry in glob(&glob_pattern).expect("Glob pattern error") {
                    match entry {
                        Ok(path) => {
                            println!("{:?}", path.display());
                            let cyclone_src = path
                                .to_str()
                                .unwrap()
                                .split("cyclonedds")
                                .collect::<Vec<&str>>();
                            let mut cyclone_src = String::from(cyclone_src[0]);
                            cyclone_src.push_str("cyclonedds");

                            paths.push(format!("{}/src/core/ddsc/include", cyclone_src));
                            paths.push(format!("{}/src/core/include", cyclone_src));

                            //
                            paths.push(format!(
                                "{}/src/core/include",
                                find_cyclone_bin_dir(cmake_bin_dir).unwrap()
                            ));

                            println!("{:?}", paths);
                            break;
                        }
                        Err(e) => println!("{:?}", e),
                    }
                }
                // now get the sysroot
                if let Ok(toolchain_sysroot) = env::var("TOOLCHAIN_SYSROOT") {
                    Some(HeaderLocation::FromCMakeEnvironment(
                        paths,
                        toolchain_sysroot,
                    ))
                } else {
                    println!("Unable to get TOOLCHAIN_SYSROOT");
                    Some(HeaderLocation::FromCMakeEnvironment(paths, "/".to_string()))
                }
            } else {
                None
            }
        } else {
            println!("No CMAKE environment or CYCLONEDDS_[LIB|INCLUDE]_DIR found");

            for target in SEARCH_TARGETS {
                let path = format!("{}/include/dds/dds.h", target);
                let path = Path::new(&path);
                if path.exists() {
                    let lib_path = format!("{}/lib", target);
                    println!("cargo:rustc-link-search={}", lib_path);
                    return Some(HeaderLocation::FromEnvironment(vec![format!(
                        "{}/include",
                        target
                    )]));
                }
            }
            //try some defaults

            println!("Cannot find dds/dds.h attempting to build");
            download();
            configure_and_build();
            let local_build_libpath = format!("{}/install/lib/libddsc.so", &outdir);
            let local_build_so = Path::new(local_build_libpath.as_str());

            if local_build_so.exists() {
                println!("cargo:rustc-link-search={}/install/lib", &outdir);
                let include_dir = format!("{}/install/include", &outdir);
                let path = format!("{}/dds/dds.h", &include_dir);
                let path = Path::new(&path);

                if path.exists() {
                    println!("Found {}", &path.to_str().unwrap());
                    let paths = vec![include_dir];
                    Some(HeaderLocation::FromLocalBuild(paths))
                } else {
                    println!("Cannot find dds/dds.h");
                    None
                }
            } else {
                None
            }
        }
    }

    fn find_cyclone_bin_dir(cmake_bin_dir: &str) -> Option<String> {
        Some(format!(
            "{}/sys/cyclonedds/src/ddsrt/include",
            cmake_bin_dir
        ))
    }

    fn add_whitelist(builder: bindgen::Builder) -> bindgen::Builder {
        builder
            .derive_default(true)
            .generate_cstr(true)
            .prepend_enum_name(false)
            // basic operations
            .allowlist_function(r"^dds_(get|read|take|write|forward)(_.+|cdr)?$")
            // create instances
            .allowlist_function(r"^dds_create_(domain|participant|publisher|subscriber|writer|read(er|condition)|topic(_sertype)?|waitset)$")
            // listener operations
            .allowlist_function(r"^dds_((.+)_listener|lset_.+|waitset_.+|triggered)$")
            // memory allocation functions
            .allowlist_function(r"^dds_((re)?alloc|free)(.+)?")
            // QoS operation and types
            .allowlist_function(r"^dds_((.+)_qos|qos_equal|q[sg]et_.+)$")
            // DDS QoS policies
            .rustified_enum(r"^dds_.+_kind$")
            // sertype,serdata operations
            .allowlist_function(r"^ddsi_ser(type|data).+$")
            // serdata hash functions
            .allowlist_function(r"^ddsrt_md5_.+$")
            // for shm feature
            .allowlist_function(r"^dds_((loan).+|.+_loan)$")
            // handling iceoryx chunks
            .allowlist_function("iceoryx_header_from_chunk")
            .allowlist_function("free_iox_chunk")
            // handling builtin topics
            .allowlist_type(r"^dds_builtintopic_.+")
            .allowlist_type(r"^dds_stream_.+")
            // UDP transport
            .allowlist_type("nn_rdata")
            // constants
            .allowlist_var(r"^DDS_(BUILTIN_TOPIC|TOPIC|OP)_.+$")
            .allowlist_var(r"^DDS_.+_(SAMPLE|VIEW|INSTANCE)_STATE$")
            .allowlist_var(r"^BUILTIN_TOPIC_DCPS.+")
            // instance status
            .allowlist_type("dds_status_id")
            .constified_enum("dds_status_id")
            // for debug
            .allowlist_function("dds_delete")
            .allowlist_function("dds_set_status_mask")
    }

    pub fn generate(include_paths: &std::vec::Vec<String>, maybe_sysroot: Option<&String>) {
        let mut bindings = bindgen::Builder::default().header("wrapper.h");

        #[cfg(feature = "shm")]
        {
            bindings = bindings.clang_arg("-DCYCLONEDDS_RS_SHM");
        }

        for path in include_paths {
            bindings = bindings.clang_arg(format!("-I{}", path));
        }

        if let Some(sysroot) = maybe_sysroot {
            bindings = bindings.clang_arg(format!("--sysroot={}", sysroot));
        }

        let bg = add_whitelist(bindings)
            .generate()
            .expect("Unable to generate bindings");

        if let Ok(path) = env::var("OUT_DIR") {
            let out_path = PathBuf::from(path);
            let bindings_path = out_path.join("generated.rs");
            bg.write_to_file(bindings_path.clone())
                .expect("Couldn't write bindings");
            fs::copy(bindings_path, PathBuf::from("src/generated.rs")).unwrap();
        } else {
            println!("OUT_DIR not set, not generating bindings");
        }
    }

    pub fn main() {
        for (key, value) in env::vars() {
            println!("{}: {}", key, value);
        }
        #[allow(unused_mut)]
        let mut headerloc = find_cyclonedds().unwrap();

        #[cfg(feature = "shm")]
        {
            if let Some(iceoryx_headers) = find_iceoryx("v2.0.2") {
                headerloc.add_paths(iceoryx_headers.get_paths());
            } else if let Some(iceoryx_headers) = find_iceoryx("v2.0.0") {
                headerloc.add_paths(iceoryx_headers.get_paths());
            }
        }

        match &headerloc {
            HeaderLocation::FromCMakeEnvironment(paths, sysroot) => generate(paths, Some(sysroot)),
            HeaderLocation::FromEnvironment(paths) | HeaderLocation::FromLocalBuild(paths) => {
                generate(paths, None)
            }
        }

        match &headerloc {
            HeaderLocation::FromCMakeEnvironment(paths, sysroot) => {
                compile_inlines(paths, Some(sysroot))
            }
            HeaderLocation::FromEnvironment(paths) | HeaderLocation::FromLocalBuild(paths) => {
                compile_inlines(paths, None)
            }
        }
    }

    fn compile_inlines(include_paths: &Vec<String>, _maybe_sysroot: Option<&String>) {
        let mut cc = cc::Build::new();

        cc.file("inline_functions.c");

        for dir in include_paths {
            cc.include(dir);
        }
        cc.compile("libinline_functions.a");
    }
}
