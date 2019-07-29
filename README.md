# `cargo local-registry`

[![Build Status](https://travis-ci.org/alexcrichton/cargo-local-registry.svg?branch=master)](https://travis-ci.org/alexcrichton/cargo-local-registry)
[![Build status](https://ci.appveyor.com/api/projects/status/x867la68pp0s94an/branch/master?svg=true)](https://ci.appveyor.com/project/alexcrichton/cargo-local-registry/branch/master)

This is a Cargo subcommand to ease maintenance of local registries. Support for
a local registry is being added in
[rust-lang/cargo#2361](https://github.com/rust-lang/cargo/pull/2361) and will be
able to redirect all Cargo downloads/requests to a registry stored locally.

This support is often useful for "offline builds" by preparing the list of all
Rust dependencies ahead of time and shipping them to a build machine in a
pre-ordained format. A local registry is an index and a collection of tarballs,
all of which currently originate from crates.io.

The purpose of this subcommand will be to manage these registries and allow
adding/deleting packages with ease.

## Installation

To install from source you can execute:

```
cargo install cargo-local-registry
```

Note that you'll need the build tools listed below for this to succeed. If you'd
prefer to download precompiled binaries assembled on the CI for this repository,
you may also use the [GitHub releases][releases]

[releases]: https://github.com/alexcrichton/cargo-local-registry/releases

## Building

As part of the build process you will need [gcc], [openssl] and [cmake] in your
`PATH`.

[gcc]: https://gcc.gnu.org/install/download.html
[openssl]: https://www.openssl.org/source/
[cmake]: https://cmake.org/download/

Afterwards you can build this repository via:

```
cargo build
```

And the resulting binary will be inside `target/debug`

## Usage

One of the primary operations will be to create a local registry from a lock
file itself. This can be done via

```
cargo local-registry --sync path/to/Cargo.lock path/to/registry
```

This command will:

* Download all dependencies from the crates.io registry
* Verify all checksums of what's downloaded
* Place all downloads in `path/to/registry`
* Prepare the index of `path/to/registry` to reflect all this information

# License

This project is licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
   http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or
   http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in cargo-local-registry by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
