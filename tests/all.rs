extern crate tempdir;

use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use std::process::Command;
use std::sync::{Once, Mutex, MutexGuard, ONCE_INIT};

use tempdir::TempDir;

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

static INIT: Once = ONCE_INIT;
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
    let td = TempDir::new("local-registry").unwrap();
    let output = run(cmd().arg(td.path()));
    assert!(td.path().join("index").exists());
    assert_eq!(output, "");
}

#[test]
fn dst_no_exists() {
    let _l = lock();
    let td = TempDir::new("local-registry").unwrap();
    let output = run(cmd().arg(td.path().join("foo")));
    assert!(td.path().join("foo/index").exists());
    assert_eq!(output, "");
}

#[test]
fn empty_cargo_lock() {
    let _l = lock();
    let td = TempDir::new("local-registry").unwrap();
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
[root]
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
    let td = TempDir::new("local-registry").unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(&td.path().join("Cargo.toml")).unwrap().write_all(br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        libc = "0.2.7"
    "#).unwrap();
    File::create(&td.path().join("src/lib.rs")).unwrap().write_all(b"").unwrap();
    File::create(&lock).unwrap().write_all(br#"
[root]
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
    run(cmd().arg(&registry).arg("--sync").arg(&lock));

    assert!(registry.join("index").is_dir());
    assert!(registry.join("index/li/bc/libc").is_file());
    assert!(registry.join("libc-0.2.7.crate").is_file());

    File::create(&lock).unwrap().write_all(br#"
[root]
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
    run(cmd().arg(&registry).arg("--sync").arg(&lock));

    assert!(registry.join("index").is_dir());
    assert!(registry.join("index/li/bc/libc").is_file());
    assert!(registry.join("libc-0.2.6.crate").is_file());
    assert!(registry.join("libc-0.2.7.crate").is_file());

    let mut contents = String::new();
    File::open(registry.join("index/li/bc/libc")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents.lines().count(), 2);
    assert!(contents.contains("0.2.6"));
    assert!(contents.contains("0.2.7"));
}

#[test]
fn git_dependency() {
    let _l = lock();
    let td = TempDir::new("local-registry").unwrap();
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
[root]
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
    let td = TempDir::new("local-registry").unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(&td.path().join("Cargo.toml")).unwrap().write_all(br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        libc = "0.2.7"
        websocket = "0.20.2"
    "#).unwrap();
    File::create(&td.path().join("src/lib.rs")).unwrap().write_all(b"").unwrap();
    File::create(&lock).unwrap().write_all(br#"
[root]
name = "foo"
version = "0.1.0"
dependencies = [
 "libc 0.2.7 (registry+https://github.com/rust-lang/crates.io-index)",
 "websocket 0.20.2 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "libc"
version = "0.2.7"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "libc"
version = "0.1.4"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "websocket"
version = "0.20.2"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = [
 "libc 0.1.4 (registry+https://github.com/rust-lang/crates.io-index)",
]
"#).unwrap();
    run(cmd().arg(&registry).arg("--sync").arg(&lock));

    let mut contents = String::new();
    File::open(registry.join("index/li/bc/libc")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"libc","vers":"0.1.4","deps":[],"cksum":"93a57b3496432ca744a67300dae196f8d4bbe33dfa7dc27adabfb6faa4643bb2","features":{"default":["cargo-build"],"cargo-build":[]},"yanked":false}
{"name":"libc","vers":"0.2.7","deps":[],"cksum":"4870ef6725dde13394134e587e4ab4eca13cb92e916209a31c851b49131d3c75","features":{"default":[]},"yanked":false}"#);

    // A lot of this doesn't look particularity ordered. It's not! This line is
    // exactly as it appears in the crates.io index. As long as crates.io
    // doesn't rewrite all the crates we should be fine.
    contents.clear();
    File::open(registry.join("index/we/bs/websocket")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"websocket","vers":"0.20.2","deps":[{"name":"byteorder","req":"^1.0","features":[],"optional":false,"default_features":true,"target":null,"kind":"normal"},{"name":"hyper","req":"^0.10.6","features":[],"optional":false,"default_features":true,"target":null,"kind":"normal"},{"name":"unicase","req":"^1.0","features":[],"optional":false,"default_features":true,"target":null,"kind":"normal"},{"name":"tokio-tls","req":"^0.1","features":[],"optional":true,"default_features":true,"target":null,"kind":"normal"},{"name":"url","req":"^1.0","features":[],"optional":false,"default_features":true,"target":null,"kind":"normal"},{"name":"bytes","req":"^0.4","features":[],"optional":true,"default_features":true,"target":null,"kind":"normal"},{"name":"tokio-core","req":"^0.1","features":[],"optional":true,"default_features":true,"target":null,"kind":"normal"},{"name":"tokio-io","req":"^0.1.2","features":[],"optional":true,"default_features":true,"target":null,"kind":"normal"},{"name":"futures","req":"^0.1","features":[],"optional":true,"default_features":true,"target":null,"kind":"normal"},{"name":"native-tls","req":"^0.1.2","features":[],"optional":true,"default_features":true,"target":null,"kind":"normal"},{"name":"sha1","req":"^0.2","features":[],"optional":false,"default_features":true,"target":null,"kind":"normal"},{"name":"rand","req":"^0.3","features":[],"optional":false,"default_features":true,"target":null,"kind":"normal"},{"name":"base64","req":"^0.5","features":[],"optional":false,"default_features":true,"target":null,"kind":"normal"},{"name":"bitflags","req":"^0.9","features":[],"optional":false,"default_features":true,"target":null,"kind":"normal"}],"cksum":"eb277e7f4c23dc49176f74ae200e77651764efb2c25f56ad2d22623b63826369","features":{"sync":[],"default":["sync","sync-ssl","async","async-ssl"],"async-ssl":["native-tls","tokio-tls","async"],"sync-ssl":["native-tls","sync"],"async":["tokio-core","tokio-io","bytes","futures"],"nightly":["hyper/nightly"]},"yanked":false}"#);

    File::create(&lock).unwrap().write_all(br#"
[root]
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
    run(cmd().arg(&registry).arg("--sync").arg(&lock));

    contents.clear();
    File::open(registry.join("index/li/bc/libc")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"libc","vers":"0.1.4","deps":[],"cksum":"93a57b3496432ca744a67300dae196f8d4bbe33dfa7dc27adabfb6faa4643bb2","features":{"default":["cargo-build"],"cargo-build":[]},"yanked":false}
{"name":"libc","vers":"0.2.6","deps":[],"cksum":"b608bf5e09bb38b075938d5d261682511bae283ef4549cc24fa66b1b8050de7b","features":{"default":[]},"yanked":false}
{"name":"libc","vers":"0.2.7","deps":[],"cksum":"4870ef6725dde13394134e587e4ab4eca13cb92e916209a31c851b49131d3c75","features":{"default":[]},"yanked":false}"#);
}

#[test]
fn git_deterministic() {
    let td = TempDir::new("local-registry").unwrap();
    let lock = td.path().join("Cargo.lock");
    let registry = td.path().join("registry");
    fs::create_dir(td.path().join("src")).unwrap();
    File::create(&td.path().join("Cargo.toml")).unwrap().write_all(br#"
        [package]
        name = "foo"
        version = "0.1.0"
        authors = []

        [dependencies]
        websocket = { git = "https://github.com/cyderize/rust-websocket" }
    "#).unwrap();
    File::create(&td.path().join("src/lib.rs")).unwrap().write_all(b"").unwrap();
    File::create(&lock).unwrap().write_all(br#"
[root]
name = "foo"
version = "0.1.0"
dependencies = [
 "websocket 0.20.2 (git+https://github.com/cyderize/rust-websocket)",
]

[[package]]
name = "websocket"
version = "0.20.2"
source = "git+https://github.com/cyderize/rust-websocket#874bba4141d651e9dce2c680bc3cccc523a6c2b3"
"#).unwrap();
    run(cmd().arg(&registry).arg("--sync").arg(&lock).arg("--git"));

    let mut contents = String::new();
    File::open(registry.join("index/we/bs/websocket")).unwrap()
        .read_to_string(&mut contents).unwrap();
    assert_eq!(contents, r#"{"name":"websocket","vers":"0.20.2","deps":[{"name":"base64","req":"^0.5","features":[],"optional":false,"default_features":true,"target":null,"kind":null},{"name":"bitflags","req":"^0.9","features":[],"optional":false,"default_features":true,"target":null,"kind":null},{"name":"byteorder","req":"^1.0","features":[],"optional":false,"default_features":true,"target":null,"kind":null},{"name":"bytes","req":"^0.4","features":[],"optional":true,"default_features":true,"target":null,"kind":null},{"name":"futures","req":"^0.1","features":[],"optional":true,"default_features":true,"target":null,"kind":null},{"name":"futures-cpupool","req":"^0.1","features":[],"optional":false,"default_features":true,"target":null,"kind":"dev"},{"name":"hyper","req":"^0.10.6","features":[],"optional":false,"default_features":true,"target":null,"kind":null},{"name":"native-tls","req":"^0.1.2","features":[],"optional":true,"default_features":true,"target":null,"kind":null},{"name":"rand","req":"^0.3","features":[],"optional":false,"default_features":true,"target":null,"kind":null},{"name":"sha1","req":"^0.2","features":[],"optional":false,"default_features":true,"target":null,"kind":null},{"name":"tokio-core","req":"^0.1","features":[],"optional":true,"default_features":true,"target":null,"kind":null},{"name":"tokio-io","req":"^0.1.2","features":[],"optional":true,"default_features":true,"target":null,"kind":null},{"name":"tokio-tls","req":"^0.1","features":[],"optional":true,"default_features":true,"target":null,"kind":null},{"name":"unicase","req":"^1.0","features":[],"optional":false,"default_features":true,"target":null,"kind":null},{"name":"url","req":"^1.0","features":[],"optional":false,"default_features":true,"target":null,"kind":null}],"features":{"async":["bytes","futures","tokio-core","tokio-io"],"async-ssl":["async","native-tls","tokio-tls"],"default":["async","async-ssl","sync","sync-ssl"],"nightly":["hyper/nightly"],"sync":[],"sync-ssl":["native-tls","sync"]},"cksum":"","yanked":null}"#);
}

fn run(cmd: &mut Command) -> String {
    let output = cmd.output().unwrap();
    if !output.status.success() {
        panic!("failed to run {:?}\n--- stdout\n{}\n--- stderr\n{}", cmd,
               String::from_utf8_lossy(&output.stdout),
               String::from_utf8_lossy(&output.stderr));
    }
    String::from_utf8_lossy(&output.stdout).into_owned()
}
