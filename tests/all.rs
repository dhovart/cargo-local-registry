extern crate tempfile;

use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use std::process::Command;
use std::sync::{Once, Mutex, MutexGuard};

use tempfile::TempDir;

fn cmd() -> Command {
    let mut me = env::current_exe().unwrap();
    me.pop();
    if me.ends_with("deps") {
        me.pop();
    }
    me.push("cargo-local-registry");
    let mut cmd = Command::new(me);
    cmd.arg("local-registry");
    return cmd
}

static INIT: Once = Once::new();
static mut LOCK: *mut Mutex<()> = 0 as *mut _;

fn lock() -> MutexGuard<'static, ()> {
    unsafe {
        INIT.call_once(|| {
            LOCK = Box::into_raw(Box::new(Mutex::new(())));
        });
        (*LOCK).lock().unwrap()
    }
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
    let output = run(cmd().arg(td.path()));
    assert!(td.path().join("index").exists());
    assert_eq!(output, "");
}

#[test]
fn dst_no_exists() {
    let _l = lock();
    let td = TempDir::new().unwrap();
    let output = run(cmd().arg(td.path().join("foo")));
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
    File::create(&td.path().join("Cargo.toml")).unwrap().write_all(br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []
    "#).unwrap();
    File::create(&td.path().join("src/lib.rs")).unwrap().write_all(b"").unwrap();
    File::create(&lock).unwrap().write_all(br#"
[[package]]
name = "foo"
version = "0.1.0"
dependencies = []
"#).unwrap();
    run(cmd().arg(&registry).arg("--sync").arg(&lock).arg("-v"));

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
    File::create(&td.path().join("Cargo.toml")).unwrap().write_all(br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        libc = "0.2.6"
    "#).unwrap();
    File::create(&td.path().join("src/lib.rs")).unwrap().write_all(b"").unwrap();
    File::create(&lock).unwrap().write_all(br#"
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
"#).unwrap();
    println!("one: {}", run(cmd().arg(&registry).arg("--sync").arg(&lock)));

    assert!(registry.join("index").is_dir());
    assert!(registry.join("index/li/bc/libc").is_file());
    assert!(registry.join("libc-0.2.7.crate").is_file());

    File::create(&lock).unwrap().write_all(br#"
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
"#).unwrap();
    println!("two: {}", run(cmd().arg(&registry).arg("--sync").arg(&lock)));

    assert!(registry.join("index").is_dir());
    assert!(registry.join("index/li/bc/libc").is_file());
    assert!(registry.join("libc-0.2.6.crate").is_file());
    assert!(!registry.join("libc-0.2.7.crate").exists());

    let mut contents = String::new();
    File::open(registry.join("index/li/bc/libc")).unwrap()
        .read_to_string(&mut contents).unwrap();
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
    File::create(&td.path().join("Cargo.toml")).unwrap().write_all(br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        libc = { git = "https://github.com/rust-lang/libc" }
    "#).unwrap();
    File::create(&td.path().join("src/lib.rs")).unwrap().write_all(b"").unwrap();
    File::create(&lock).unwrap().write_all(br#"
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
"#).unwrap();
    run(cmd().arg(&registry).arg("--sync").arg(&lock).arg("--git"));

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
    File::create(&td.path().join("Cargo.toml")).unwrap().write_all(br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        libc = "0.1.4"
        filetime = "0.1.10"
    "#).unwrap();
    File::create(&td.path().join("src/lib.rs")).unwrap().write_all(b"").unwrap();
    File::create(&lock).unwrap().write_all(br#"
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
"#).unwrap();
    run(cmd().arg(&registry).arg("--sync").arg(&lock));

    let mut contents = String::new();
    File::open(registry.join("index/li/bc/libc")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"libc","vers":"0.1.4","deps":[],"cksum":"93a57b3496432ca744a67300dae196f8d4bbe33dfa7dc27adabfb6faa4643bb2","features":{"cargo-build":[],"default":["cargo-build"]},"yanked":false}
{"name":"libc","vers":"0.2.7","deps":[],"cksum":"4870ef6725dde13394134e587e4ab4eca13cb92e916209a31c851b49131d3c75","features":{"default":[]},"yanked":false}"#);

    // A lot of this doesn't look particularity ordered. It's not! This line is
    // exactly as it appears in the crates.io index. As long as crates.io
    // doesn't rewrite all the crates we should be fine.
    contents.clear();
    File::open(registry.join("index/fi/le/filetime")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"filetime","vers":"0.1.10","deps":[{"name":"libc","req":"^0.2","features":[],"optional":false,"default_features":true,"target":null,"kind":null,"package":null},{"name":"tempdir","req":"^0.3","features":[],"optional":false,"default_features":true,"target":null,"kind":"dev","package":null}],"cksum":"5363ab8e4139b8568a6237db5248646e5a8a2f89bd5ccb02092182b11fd3e922","features":{},"yanked":false}"#);

    File::create(&lock).unwrap().write_all(br#"
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
"#).unwrap();
    run(cmd().arg(&registry).arg("--sync").arg(&lock));

    contents.clear();
    File::open(registry.join("index/li/bc/libc")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"libc","vers":"0.1.4","deps":[],"cksum":"93a57b3496432ca744a67300dae196f8d4bbe33dfa7dc27adabfb6faa4643bb2","features":{"cargo-build":[],"default":["cargo-build"]},"yanked":false}
{"name":"libc","vers":"0.2.6","deps":[],"cksum":"b608bf5e09bb38b075938d5d261682511bae283ef4549cc24fa66b1b8050de7b","features":{"default":[]},"yanked":false}"#);
}

#[test]
fn lowercased() {
    let td = TempDir::new().unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(&td.path().join("Cargo.toml")).unwrap().write_all(br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        Inflector = "0.11.3"
    "#).unwrap();
    File::create(&td.path().join("src/lib.rs")).unwrap().write_all(b"").unwrap();
    File::create(&lock).unwrap().write_all(br#"
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
"#).unwrap();
    run(cmd().arg(&registry).arg("--sync").arg(&lock));

    let mut contents = String::new();
    let path = registry.join("index/in/fl/inflector");
    let path = fs::canonicalize(path).unwrap();

    assert_eq!(path.file_name().unwrap(), "inflector");

    File::open(registry.join("index/in/fl/inflector")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"Inflector","vers":"0.11.3","deps":[{"name":"lazy_static","req":"^1.0.0","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"regex","req":"^1.0","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"4467f98bb61f615f8273359bf1c989453dfc1ea4a45ae9298f1dcd0672febe5d","features":{"default":["heavyweight"],"heavyweight":["lazy_static","regex"],"lazy_static":["dep:lazy_static"],"regex":["dep:regex"],"unstable":[]},"yanked":false}"#);
}

#[test]
fn renamed() {
    let td = TempDir::new().unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(&td.path().join("Cargo.toml")).unwrap().write_all(br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        rustc-demangle = "0.1.14"
    "#).unwrap();
    File::create(&td.path().join("src/lib.rs")).unwrap().write_all(b"").unwrap();
    File::create(&lock).unwrap().write_all(br#"
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
"#).unwrap();
    run(cmd().arg(&registry).arg("--sync").arg(&lock));

    let mut contents = String::new();
    let path = registry.join("index/ru/st/rustc-demangle");
    let path = fs::canonicalize(path).unwrap();

    assert_eq!(path.file_name().unwrap(), "rustc-demangle");

    File::open(registry.join("index/ru/st/rustc-demangle")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"rustc-demangle","vers":"0.1.14","deps":[{"name":"compiler_builtins","req":"^0.1.2","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"core","req":"^1.0.0","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":"rustc-std-workspace-core"}],"cksum":"ccc78bfd5acd7bf3e89cffcf899e5cb1a52d6fafa8dec2739ad70c9577a57288","features":{"compiler_builtins":["dep:compiler_builtins"],"core":["dep:core"],"rustc-dep-of-std":["compiler_builtins","core"]},"yanked":false}"#);
}

#[test]
fn clean_mode() {
    let td = TempDir::new().unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(&td.path().join("Cargo.toml")).unwrap().write_all(br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        lazy_static = "0.2.11"
        language-tags = "0.2.2"
    "#).unwrap();
    File::create(&td.path().join("src/lib.rs")).unwrap().write_all(b"").unwrap();
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
    run(cmd().arg(&registry).arg("--sync").arg(&lock));

    assert!(registry.join("language-tags-0.2.2.crate").exists());
    assert!(registry.join("lazy_static-0.2.11.crate").exists());

    let mut contents = String::new();
    File::open(registry.join("index/la/zy/lazy_static")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"lazy_static","vers":"0.2.11","deps":[{"name":"compiletest_rs","req":"^0.3","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"spin","req":"^0.4.6","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"76f033c7ad61445c5b347c7382dd1237847eb1bce590fe50365dcb33d546be73","features":{"compiletest":["compiletest_rs"],"compiletest_rs":["dep:compiletest_rs"],"nightly":[],"spin":["dep:spin"],"spin_no_std":["nightly","spin"]},"yanked":false}"#);

    contents.clear();
    File::open(registry.join("index/la/ng/language-tags")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"language-tags","vers":"0.2.2","deps":[{"name":"heapsize","req":">=0.2.2, <0.4","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"heapsize_plugin","req":"^0.1.2","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"a91d884b6667cd606bb5a69aa0c99ba811a115fc68915e7056ec08a46e93199a","features":{"heap_size":["heapsize","heapsize_plugin"],"heapsize":["dep:heapsize"],"heapsize_plugin":["dep:heapsize_plugin"]},"yanked":false}"#);

    // Modify the Cargo.toml to swap an existing library, add a new one and delete another
    File::create(&td.path().join("Cargo.toml")).unwrap().write_all(br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        lazy_static = "1.2.0"
        lazycell = "1.2.1"
    "#).unwrap();

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
    run(cmd().arg(&registry).arg("--no-delete").arg("--sync").arg(&lock));

    assert!(registry.join("language-tags-0.2.2.crate").exists());
    assert!(registry.join("lazy_static-0.2.11.crate").exists());
    assert!(registry.join("lazy_static-1.2.0.crate").exists());
    assert!(registry.join("lazycell-1.2.1.crate").exists());

    contents.clear();
    File::open(registry.join("index/la/zy/lazy_static")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"lazy_static","vers":"0.2.11","deps":[{"name":"compiletest_rs","req":"^0.3","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"spin","req":"^0.4.6","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"76f033c7ad61445c5b347c7382dd1237847eb1bce590fe50365dcb33d546be73","features":{"compiletest":["compiletest_rs"],"compiletest_rs":["dep:compiletest_rs"],"nightly":[],"spin":["dep:spin"],"spin_no_std":["nightly","spin"]},"yanked":false}
{"name":"lazy_static","vers":"1.2.0","deps":[{"name":"spin","req":"^0.4.10","features":["once"],"optional":true,"default_features":false,"target":null,"kind":null,"package":null}],"cksum":"a374c89b9db55895453a74c1e38861d9deec0b01b405a82516e9d5de4820dea1","features":{"nightly":[],"spin":["dep:spin"],"spin_no_std":["spin"]},"yanked":false}"#);
        
    contents.clear();
    File::open(registry.join("index/la/zy/lazycell")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"lazycell","vers":"1.2.1","deps":[{"name":"clippy","req":"^0.0","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"b294d6fa9ee409a054354afc4352b0b9ef7ca222c69b8812cbea9e7d2bf3783f","features":{"clippy":["dep:clippy"],"nightly":[],"nightly-testing":["clippy","nightly"]},"yanked":false}"#);

    contents.clear();
    File::open(registry.join("index/la/ng/language-tags")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"language-tags","vers":"0.2.2","deps":[{"name":"heapsize","req":">=0.2.2, <0.4","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null},{"name":"heapsize_plugin","req":"^0.1.2","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"a91d884b6667cd606bb5a69aa0c99ba811a115fc68915e7056ec08a46e93199a","features":{"heap_size":["heapsize","heapsize_plugin"],"heapsize":["dep:heapsize"],"heapsize_plugin":["dep:heapsize_plugin"]},"yanked":false}"#);

    // Run for the third time -- delete unused (default)
    run(cmd().arg(&registry).arg("--sync").arg(&lock));

    // should be deleted
    assert!(!registry.join("language-tags-0.2.2.crate").exists());
    // should be deleted
    assert!(!registry.join("lazy_static-0.2.11.crate").exists());
    assert!(registry.join("lazy_static-1.2.0.crate").exists());
    assert!(registry.join("lazycell-1.2.1.crate").exists());

    // index and its parent directory should be cleaned, too
    assert!(!registry.join("index").join("la").join("ng").exists());

    contents.clear();
    File::open(registry.join("index/la/zy/lazy_static")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"lazy_static","vers":"1.2.0","deps":[{"name":"spin","req":"^0.4.10","features":["once"],"optional":true,"default_features":false,"target":null,"kind":null,"package":null}],"cksum":"a374c89b9db55895453a74c1e38861d9deec0b01b405a82516e9d5de4820dea1","features":{"nightly":[],"spin":["dep:spin"],"spin_no_std":["spin"]},"yanked":false}"#);

    contents.clear();
    File::open(registry.join("index/la/zy/lazycell")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"lazycell","vers":"1.2.1","deps":[{"name":"clippy","req":"^0.0","features":[],"optional":true,"default_features":true,"target":null,"kind":null,"package":null}],"cksum":"b294d6fa9ee409a054354afc4352b0b9ef7ca222c69b8812cbea9e7d2bf3783f","features":{"clippy":["dep:clippy"],"nightly":[],"nightly-testing":["clippy","nightly"]},"yanked":false}"#);
}

fn run(cmd: &mut Command) -> String {
    let output = cmd.env("RUST_BACKTRACE", "1").output().unwrap();
    if !output.status.success() {
        panic!("failed to run {:?}\n--- stdout\n{}\n--- stderr\n{}", cmd,
               String::from_utf8_lossy(&output.stdout),
               String::from_utf8_lossy(&output.stderr));
    }
    String::from_utf8_lossy(&output.stdout).into_owned()
}
