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

fn run(cmd: &mut Command) -> String {
    let output = cmd.output().unwrap();
    if !output.status.success() {
        panic!("failed to run {:?}\n--- stdout\n{}\n--- stderr\n{}", cmd,
               String::from_utf8_lossy(&output.stdout),
               String::from_utf8_lossy(&output.stderr));
    }
    String::from_utf8_lossy(&output.stdout).into_owned()
}
