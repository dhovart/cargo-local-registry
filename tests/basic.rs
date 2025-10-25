extern crate tempfile;

use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once};

use serde_json::Value;
use tempfile::TempDir;

pub static INIT: Once = Once::new();
pub static mut LOCK: *mut Mutex<()> = 0 as *mut _;

pub fn lock() -> MutexGuard<'static, ()> {
    unsafe {
        INIT.call_once(|| {
            LOCK = Box::into_raw(Box::new(Mutex::new(())));
        });
        (*LOCK).lock().unwrap()
    }
}

fn cmd() -> Command {
    let mut me = env::current_exe().unwrap();
    me.pop();
    if me.ends_with("deps") {
        me.pop();
    }
    me.push("cargo-local-registry");
    Command::new(me)
}

#[test]
fn help() {
    run(cmd().arg("--help"));
    run(cmd().arg("-h"));
}

#[test]
fn no_sync() {
    let _l = lock();
    let td = TempDir::new().unwrap();
    let output = run(cmd().arg("create").arg(td.path()));
    assert!(td.path().join("index").exists());
    assert_eq!(output, "");
}

#[test]
fn dst_no_exists() {
    let _l = lock();
    let td = TempDir::new().unwrap();
    let output = run(cmd().arg("create").arg(td.path().join("foo")));
    assert!(td.path().join("foo/index").exists());
    assert_eq!(output, "");
}

#[test]
fn empty_cargo_lock() {
    let _l = lock();
    let td = TempDir::new().unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(td.path().join("Cargo.toml"))
        .unwrap()
        .write_all(
            br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []
    "#,
        )
        .unwrap();
    File::create(td.path().join("src/lib.rs"))
        .unwrap()
        .write_all(b"")
        .unwrap();
    File::create(&lock)
        .unwrap()
        .write_all(
            br#"
[[package]]
name = "foo"
version = "0.1.0"
dependencies = []
"#,
        )
        .unwrap();
    run(cmd()
        .arg("-v")
        .arg("create")
        .arg(&registry)
        .arg("--sync")
        .arg(&lock));

    assert!(registry.join("index").is_dir());
    assert_eq!(registry.join("index").read_dir().unwrap().count(), 0);
}

#[test]
fn libc_dependency() {
    let _l = lock();
    let td = TempDir::new().unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(td.path().join("Cargo.toml"))
        .unwrap()
        .write_all(
            br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        libc = "0.2.6"
    "#,
        )
        .unwrap();
    File::create(td.path().join("src/lib.rs"))
        .unwrap()
        .write_all(b"")
        .unwrap();
    File::create(&lock)
        .unwrap()
        .write_all(
            br#"
[[package]]
name = "foo"
version = "0.1.0"
dependencies = [
 "libc 0.2.7 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "libc"
version = "0.2.7"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#,
        )
        .unwrap();
    println!(
        "one: {}",
        run(cmd().arg("create").arg(&registry).arg("--sync").arg(&lock))
    );

    assert!(registry.join("index").is_dir());
    assert!(registry.join("index/li/bc/libc").is_file());
    assert!(registry.join("libc-0.2.7.crate").is_file());

    File::create(&lock)
        .unwrap()
        .write_all(
            br#"
[[package]]
name = "foo"
version = "0.1.0"
dependencies = [
 "libc 0.2.6 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "libc"
version = "0.2.6"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#,
        )
        .unwrap();
    println!(
        "two: {}",
        run(cmd().arg("create").arg(&registry).arg("--sync").arg(&lock))
    );

    assert!(registry.join("index").is_dir());
    assert!(registry.join("index/li/bc/libc").is_file());
    assert!(registry.join("libc-0.2.6.crate").is_file());
    assert!(!registry.join("libc-0.2.7.crate").exists());

    let mut contents = String::new();
    File::open(registry.join("index/li/bc/libc"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(contents.lines().count(), 1);
    assert!(contents.contains("0.2.6"));
}

#[test]
fn git_dependency() {
    let _l = lock();
    let td = TempDir::new().unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(td.path().join("Cargo.toml"))
        .unwrap()
        .write_all(
            br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        libc = { git = "https://github.com/rust-lang/libc" }
    "#,
        )
        .unwrap();
    File::create(td.path().join("src/lib.rs"))
        .unwrap()
        .write_all(b"")
        .unwrap();
    File::create(&lock)
        .unwrap()
        .write_all(
            br#"
[[package]]
name = "foo"
version = "0.1.0"
dependencies = [
 "libc 0.2.16 (git+https://github.com/rust-lang/libc)",
]

[[package]]
name = "libc"
version = "0.2.16"
source = "git+https://github.com/rust-lang/libc#36bec35aeb600bb1b8b47f4985a84a8d4a261747"
"#,
        )
        .unwrap();
    run(cmd()
        .arg("--git")
        .arg("create")
        .arg(&registry)
        .arg("--sync")
        .arg(&lock));

    assert!(registry.join("index").is_dir());
    assert!(registry.join("index/li/bc/libc").is_file());
    assert!(registry.join("libc-0.2.16.crate").is_file());
}

#[test]
fn deterministic() {
    let td = TempDir::new().unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(td.path().join("Cargo.toml"))
        .unwrap()
        .write_all(
            br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        libc = "0.1.4"
        filetime = "0.1.10"
    "#,
        )
        .unwrap();
    File::create(td.path().join("src/lib.rs"))
        .unwrap()
        .write_all(b"")
        .unwrap();
    File::create(&lock)
        .unwrap()
        .write_all(
            br#"
[[package]]
name = "foo"
version = "0.1.0"
dependencies = [
 "libc 0.1.4 (registry+https://github.com/rust-lang/crates.io-index)",
 "filetime 0.1.10 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "libc"
version = "0.1.4"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "libc"
version = "0.2.7"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "filetime"
version = "0.1.10"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = [
 "libc 0.2.7 (registry+https://github.com/rust-lang/crates.io-index)",
]
"#,
        )
        .unwrap();
    run(cmd().arg("create").arg(&registry).arg("--sync").arg(&lock));

    let mut contents = String::new();
    File::open(registry.join("index/li/bc/libc"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"libc","vers":"0.1.4","deps":[],"cksum":"93a57b3496432ca744a67300dae196f8d4bbe33dfa7dc27adabfb6faa4643bb2","features":{"cargo-build":[],"default":["cargo-build"]},"yanked":false}
{"name":"libc","vers":"0.2.7","deps":[],"cksum":"4870ef6725dde13394134e587e4ab4eca13cb92e916209a31c851b49131d3c75","features":{"default":[]},"yanked":false}"#
    );

    // A lot of this doesn't look particularity ordered. It's not! This line is
    // exactly as it appears in the crates.io index. As long as crates.io
    // doesn't rewrite all the crates we should be fine.
    contents.clear();
    File::open(registry.join("index/fi/le/filetime"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"filetime","vers":"0.1.10","deps":[{"name":"libc","req":"^0.2","features":[],"optional":false,"default_features":true,"target":null,"kind":null,"package":null},{"name":"tempdir","req":"^0.3","features":[],"optional":false,"default_features":true,"target":null,"kind":"dev","package":null}],"cksum":"5363ab8e4139b8568a6237db5248646e5a8a2f89bd5ccb02092182b11fd3e922","features":{},"yanked":false}"#
    );

    File::create(&lock)
        .unwrap()
        .write_all(
            br#"
[[package]]
name = "foo"
version = "0.1.0"
dependencies = [
 "libc 0.1.4 (registry+https://github.com/rust-lang/crates.io-index)",
 "filetime 0.1.10 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "libc"
version = "0.1.4"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "libc"
version = "0.2.6"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "filetime"
version = "0.1.10"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = [
 "libc 0.2.6 (registry+https://github.com/rust-lang/crates.io-index)",
]
"#,
        )
        .unwrap();
    run(cmd().arg("create").arg(&registry).arg("--sync").arg(&lock));

    contents.clear();
    File::open(registry.join("index/li/bc/libc"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"libc","vers":"0.1.4","deps":[],"cksum":"93a57b3496432ca744a67300dae196f8d4bbe33dfa7dc27adabfb6faa4643bb2","features":{"cargo-build":[],"default":["cargo-build"]},"yanked":false}
{"name":"libc","vers":"0.2.6","deps":[],"cksum":"b608bf5e09bb38b075938d5d261682511bae283ef4549cc24fa66b1b8050de7b","features":{"default":[]},"yanked":false}"#
    );
}

#[test]
fn lowercased() {
    let td = TempDir::new().unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(td.path().join("Cargo.toml"))
        .unwrap()
        .write_all(
            br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        Inflector = "0.11.3"
    "#,
        )
        .unwrap();
    File::create(td.path().join("src/lib.rs"))
        .unwrap()
        .write_all(b"")
        .unwrap();
    File::create(&lock)
        .unwrap()
        .write_all(
            br#"
[[package]]
name = "foo"
version = "0.1.0"
dependencies = [
 "Inflector 0.11.3 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "Inflector"
version = "0.11.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#,
        )
        .unwrap();
    run(cmd().arg("create").arg(&registry).arg("--sync").arg(&lock));

    let mut contents = String::new();
    let path = registry.join("index/in/fl/inflector");
    let path = fs::canonicalize(path).unwrap();

    assert_eq!(path.file_name().unwrap(), "inflector");

    File::open(registry.join("index/in/fl/inflector"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"Inflector","vers":"0.11.3","deps":[{"name":"lazy_static","req":"^1.0.0","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"regex","req":"^1.0","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"4467f98bb61f615f8273359bf1c989453dfc1ea4a45ae9298f1dcd0672febe5d","features":{"default":["heavyweight"],"heavyweight":["lazy_static","regex"],"lazy_static":["dep:lazy_static"],"regex":["dep:regex"],"unstable":[]},"yanked":false}"#
    );
}

#[test]
fn renamed() {
    let td = TempDir::new().unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(td.path().join("Cargo.toml"))
        .unwrap()
        .write_all(
            br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        rustc-demangle = "0.1.14"
    "#,
        )
        .unwrap();
    File::create(td.path().join("src/lib.rs"))
        .unwrap()
        .write_all(b"")
        .unwrap();
    File::create(&lock)
        .unwrap()
        .write_all(
            br#"
[[package]]
name = "foo"
version = "0.1.0"
dependencies = [
 "rustc-demangle 0.1.14 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "rustc-demangle"
version = "0.1.14"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#,
        )
        .unwrap();
    run(cmd().arg("create").arg(&registry).arg("--sync").arg(&lock));

    let mut contents = String::new();
    let path = registry.join("index/ru/st/rustc-demangle");
    let path = fs::canonicalize(path).unwrap();

    assert_eq!(path.file_name().unwrap(), "rustc-demangle");

    File::open(registry.join("index/ru/st/rustc-demangle"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"rustc-demangle","vers":"0.1.14","deps":[{"name":"compiler_builtins","req":"^0.1.2","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"core","req":"^1.0.0","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":"rustc-std-workspace-core"}],"cksum":"ccc78bfd5acd7bf3e89cffcf899e5cb1a52d6fafa8dec2739ad70c9577a57288","features":{"compiler_builtins":["dep:compiler_builtins"],"core":["dep:core"],"rustc-dep-of-std":["compiler_builtins","core"]},"yanked":false}"#
    );
}

#[test]
fn clean_mode() {
    let td = TempDir::new().unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(td.path().join("Cargo.toml"))
        .unwrap()
        .write_all(
            br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        lazy_static = "0.2.11"
        language-tags = "0.2.2"
    "#,
        )
        .unwrap();
    File::create(td.path().join("src/lib.rs"))
        .unwrap()
        .write_all(b"")
        .unwrap();
    File::create(&lock).unwrap().write_all(br#"
[[package]]
name = "foo"
version = "0.1.0"
dependencies = [
 "language-tags 0.2.2 (registry+https://github.com/rust-lang/crates.io-index)",
 "lazy_static 0.2.11 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "language-tags"
version = "0.2.2"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "lazy_static"
version = "0.2.11"
source = "registry+https://github.com/rust-lang/crates.io-index"

[metadata]
"checksum language-tags 0.2.2 (registry+https://github.com/rust-lang/crates.io-index)" = "a91d884b6667cd606bb5a69aa0c99ba811a115fc68915e7056ec08a46e93199a"
"checksum lazy_static 0.2.11 (registry+https://github.com/rust-lang/crates.io-index)" = "76f033c7ad61445c5b347c7382dd1237847eb1bce590fe50365dcb33d546be73"
"#).unwrap();
    run(cmd().arg("create").arg(&registry).arg("--sync").arg(&lock));

    assert!(registry.join("language-tags-0.2.2.crate").exists());
    assert!(registry.join("lazy_static-0.2.11.crate").exists());

    let mut contents = String::new();
    File::open(registry.join("index/la/zy/lazy_static"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"lazy_static","vers":"0.2.11","deps":[{"name":"compiletest_rs","req":"^0.3","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"spin","req":"^0.4.6","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"76f033c7ad61445c5b347c7382dd1237847eb1bce590fe50365dcb33d546be73","features":{"compiletest":["compiletest_rs"],"compiletest_rs":["dep:compiletest_rs"],"nightly":[],"spin":["dep:spin"],"spin_no_std":["nightly","spin"]},"yanked":false}"#
    );

    contents.clear();
    File::open(registry.join("index/la/ng/language-tags"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"language-tags","vers":"0.2.2","deps":[{"name":"heapsize","req":">=0.2.2, <0.4","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"heapsize_plugin","req":"^0.1.2","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"a91d884b6667cd606bb5a69aa0c99ba811a115fc68915e7056ec08a46e93199a","features":{"heap_size":["heapsize","heapsize_plugin"],"heapsize":["dep:heapsize"],"heapsize_plugin":["dep:heapsize_plugin"]},"yanked":false}"#
    );

    // Modify the Cargo.toml to swap an existing library, add a new one and delete another
    File::create(td.path().join("Cargo.toml"))
        .unwrap()
        .write_all(
            br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        lazy_static = "1.2.0"
        lazycell = "1.2.1"
    "#,
        )
        .unwrap();

    File::create(&lock).unwrap().write_all(br#"
[[package]]
name = "foo"
version = "0.1.0"
dependencies = [
 "lazy_static 1.2.0 (registry+https://github.com/rust-lang/crates.io-index)",
 "lazycell 1.2.1 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "lazy_static"
version = "1.2.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "lazycell"
version = "1.2.1"
source = "registry+https://github.com/rust-lang/crates.io-index"

[metadata]
"checksum lazy_static 1.2.0 (registry+https://github.com/rust-lang/crates.io-index)" = "a374c89b9db55895453a74c1e38861d9deec0b01b405a82516e9d5de4820dea1"
"checksum lazycell 1.2.1 (registry+https://github.com/rust-lang/crates.io-index)" = "b294d6fa9ee409a054354afc4352b0b9ef7ca222c69b8812cbea9e7d2bf3783f"
"#).unwrap();

    // Run again -- no delete unused
    run(cmd()
        .arg("--no-delete")
        .arg("create")
        .arg(&registry)
        .arg("--sync")
        .arg(&lock));

    assert!(registry.join("language-tags-0.2.2.crate").exists());
    assert!(registry.join("lazy_static-0.2.11.crate").exists());
    assert!(registry.join("lazy_static-1.2.0.crate").exists());
    assert!(registry.join("lazycell-1.2.1.crate").exists());

    contents.clear();
    File::open(registry.join("index/la/zy/lazy_static"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"lazy_static","vers":"0.2.11","deps":[{"name":"compiletest_rs","req":"^0.3","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"spin","req":"^0.4.6","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"76f033c7ad61445c5b347c7382dd1237847eb1bce590fe50365dcb33d546be73","features":{"compiletest":["compiletest_rs"],"compiletest_rs":["dep:compiletest_rs"],"nightly":[],"spin":["dep:spin"],"spin_no_std":["nightly","spin"]},"yanked":false}
{"name":"lazy_static","vers":"1.2.0","deps":[{"name":"spin","req":"^0.4.10","features":["once"],"optional":true,"default_features":false,"target":null,"kind":null,"package":null}],"cksum":"a374c89b9db55895453a74c1e38861d9deec0b01b405a82516e9d5de4820dea1","features":{"nightly":[],"spin":["dep:spin"],"spin_no_std":["spin"]},"yanked":false}"#
    );

    contents.clear();
    File::open(registry.join("index/la/zy/lazycell"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"lazycell","vers":"1.2.1","deps":[{"name":"clippy","req":"^0.0","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"b294d6fa9ee409a054354afc4352b0b9ef7ca222c69b8812cbea9e7d2bf3783f","features":{"clippy":["dep:clippy"],"nightly":[],"nightly-testing":["clippy","nightly"]},"yanked":false}"#
    );

    contents.clear();
    File::open(registry.join("index/la/ng/language-tags"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"language-tags","vers":"0.2.2","deps":[{"name":"heapsize","req":">=0.2.2, <0.4","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"heapsize_plugin","req":"^0.1.2","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"a91d884b6667cd606bb5a69aa0c99ba811a115fc68915e7056ec08a46e93199a","features":{"heap_size":["heapsize","heapsize_plugin"],"heapsize":["dep:heapsize"],"heapsize_plugin":["dep:heapsize_plugin"]},"yanked":false}"#
    );

    // Run for the third time -- delete unused (default)
    run(cmd().arg("create").arg(&registry).arg("--sync").arg(&lock));

    // should be deleted
    assert!(!registry.join("language-tags-0.2.2.crate").exists());
    // should be deleted
    assert!(!registry.join("lazy_static-0.2.11.crate").exists());
    assert!(registry.join("lazy_static-1.2.0.crate").exists());
    assert!(registry.join("lazycell-1.2.1.crate").exists());

    // index and its parent directory should be cleaned, too
    assert!(!registry.join("index").join("la").join("ng").exists());

    contents.clear();
    File::open(registry.join("index/la/zy/lazy_static"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"lazy_static","vers":"1.2.0","deps":[{"name":"spin","req":"^0.4.10","features":["once"],"optional":true,"default_features":false,"target":null,"kind":null,"package":null}],"cksum":"a374c89b9db55895453a74c1e38861d9deec0b01b405a82516e9d5de4820dea1","features":{"nightly":[],"spin":["dep:spin"],"spin_no_std":["spin"]},"yanked":false}"#
    );

    contents.clear();
    File::open(registry.join("index/la/zy/lazycell"))
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(
        contents,
        r#"{"name":"lazycell","vers":"1.2.1","deps":[{"name":"clippy","req":"^0.0","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"b294d6fa9ee409a054354afc4352b0b9ef7ca222c69b8812cbea9e7d2bf3783f","features":{"clippy":["dep:clippy"],"nightly":[],"nightly-testing":["clippy","nightly"]},"yanked":false}"#
    );
}

fn run(cmd: &mut Command) -> String {
    let output = cmd.env("RUST_BACKTRACE", "1").output().unwrap();
    if !output.status.success() {
        panic!(
            "failed to run {:?}\n--- stdout\n{}\n--- stderr\n{}",
            cmd,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn test_checksum_compatibility_with_cratesio() {
    let _l = lock();

    // Create a temporary directory for our test registry
    let td = TempDir::new().unwrap();
    let registry = td.path().join("registry");
    let lock = td.path().join("Cargo.lock");
    let manifest = td.path().join("Cargo.toml");

    // Create src directory and lib.rs
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(td.path().join("src/lib.rs"))
        .unwrap()
        .write_all(b"")
        .unwrap();

    // Create a sample Cargo.toml
    File::create(&manifest)
        .unwrap()
        .write_all(
            br#"
[package]
name = "test-app"
version = "0.1.0"

[dependencies]
serde_json = "1.0.128"
"#,
        )
        .unwrap();

    // Create a sample Cargo.lock with some real dependencies
    File::create(&lock)
        .unwrap()
        .write_all(
            br#"
# This file is automatically @generated by Cargo.
# It is not intended for manual editing.
version = 3

[[package]]
name = "serde"
version = "1.0.210"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "c8e3592472072e6e22e0a54d5904d9febf8508f65fb8552499a1abc7d1078c3a"

[[package]]
name = "serde_json"
version = "1.0.128"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "6ff5456707a1de34e7e37f2a6fd3d3f808c318259cbd01ab6377795054b483d8"
dependencies = [
 "serde",
]

[[package]]
name = "test-app"
version = "0.1.0"
dependencies = [
 "serde_json",
]
"#,
        )
        .unwrap();

    // Sync from the Cargo.lock file to create our local registry
    run(cmd().arg("create").arg(&registry).arg("--sync").arg(&lock));

    // Verify that the checksums in our local registry match what was in Cargo.lock
    verify_checksums_match_lock_file(&registry, &lock);
}

fn verify_checksums_match_lock_file(registry_path: &Path, lock_path: &Path) {
    // Parse the Cargo.lock file to extract expected checksums
    let lock_content = fs::read_to_string(lock_path).unwrap();
    let expected_checksums = parse_lock_file_checksums(&lock_content);

    // Check each crate in our local registry
    for (name_version, expected_checksum) in &expected_checksums {
        let parts: Vec<&str> = name_version.split(':').collect();
        if parts.len() != 2 {
            continue;
        }
        let (crate_name, version) = (parts[0], parts[1]);

        // Read the local index file
        let crate_path = get_crate_index_path(registry_path, crate_name);
        if !crate_path.exists() {
            panic!("Local index file not found for crate: {}", crate_name);
        }

        let index_content = fs::read_to_string(&crate_path).unwrap();

        // Find the specific version in the index
        let mut found_checksum = None;
        for line in index_content.lines() {
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(parsed) = serde_json::from_str::<Value>(line) {
                if let (Some(vers), Some(cksum)) = (
                    parsed.get("vers").and_then(|v| v.as_str()),
                    parsed.get("cksum").and_then(|c| c.as_str()),
                ) {
                    if vers == version {
                        found_checksum = Some(cksum.to_string());
                        break;
                    }
                }
            }
        }

        match found_checksum {
            Some(actual_checksum) => {
                assert_eq!(
                    &actual_checksum, expected_checksum,
                    "Checksum mismatch for {}:{}\n  Expected: {}\n  Actual:   {}",
                    crate_name, version, expected_checksum, actual_checksum
                );
            }
            None => {
                panic!(
                    "Version {} not found in local registry for crate {}",
                    version, crate_name
                );
            }
        }
    }
}

fn parse_lock_file_checksums(content: &str) -> HashMap<String, String> {
    let mut checksums = HashMap::new();
    let mut current_name = None;
    let mut current_version = None;

    for line in content.lines() {
        let line = line.trim();

        if line.starts_with("name = ") {
            current_name = Some(line[7..].trim_matches('"').to_string());
        } else if line.starts_with("version = ") {
            current_version = Some(line[10..].trim_matches('"').to_string());
        } else if line.starts_with("checksum = ") {
            if let (Some(name), Some(version)) = (&current_name, &current_version) {
                let checksum = line[11..].trim_matches('"').to_string();
                checksums.insert(format!("{}:{}", name, version), checksum);
            }
        } else if line.starts_with("[[package]]") {
            // Reset for next package
            current_name = None;
            current_version = None;
        }
    }

    checksums
}
fn get_crate_index_path(registry_path: &Path, crate_name: &str) -> std::path::PathBuf {
    let index_path = registry_path.join("index");

    match crate_name.len() {
        1 => index_path.join("1").join(crate_name),
        2 => index_path.join("2").join(crate_name),
        3 => index_path
            .join("3")
            .join(&crate_name[0..1])
            .join(crate_name),
        _ => {
            let prefix = format!("{}/{}", &crate_name[0..2], &crate_name[2..4]);
            index_path.join(prefix).join(crate_name)
        }
    }
}
