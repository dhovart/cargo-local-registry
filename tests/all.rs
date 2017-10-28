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
    assert_eq!(contents, r#"{"name":"filetime","vers":"0.1.10","deps":[{"name":"libc","req":"^0.2","features":[],"optional":false,"default_features":true,"target":null,"kind":null},{"name":"tempdir","req":"^0.3","features":[],"optional":false,"default_features":true,"target":null,"kind":"dev"}],"cksum":"5363ab8e4139b8568a6237db5248646e5a8a2f89bd5ccb02092182b11fd3e922","features":{},"yanked":false}"#);

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
{"name":"libc","vers":"0.2.6","deps":[],"cksum":"b608bf5e09bb38b075938d5d261682511bae283ef4549cc24fa66b1b8050de7b","features":{"default":[]},"yanked":false}
{"name":"libc","vers":"0.2.7","deps":[],"cksum":"4870ef6725dde13394134e587e4ab4eca13cb92e916209a31c851b49131d3c75","features":{"default":[]},"yanked":false}"#);
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
