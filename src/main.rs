extern crate cargo;
extern crate rustc_serialize;

use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::Path;

use cargo::core::{SourceId, Dependency};
use cargo::CliResult;
use cargo::util::{human, ChainError, ToUrl, Config, CargoResult};

#[derive(RustcDecodable)]
struct Options {
    arg_path: String,
    flag_sync: Option<String>,
    flag_host: Option<String>,
    flag_verbose: bool,
    flag_quiet: bool,
    flag_color: Option<String>,
}

#[derive(RustcDecodable)]
struct RegistryPackage {
    vers: String,
}

fn main() {
    cargo::execute_main_without_stdin(real_main, false, r#"
Vendor all dependencies for a project locally

Usage:
    cargo local-registry [options] [<path>]

Options:
    -h, --help               Print this message
    -s, --sync LOCK          Sync the registry with LOCK
    --host HOST              Registry index to sync with
    -v, --verbose            Use verbose output
    -q, --quiet              No output printed to stdout
    --color WHEN             Coloring: auto, always, never
"#)
}

fn real_main(options: Options, config: &Config) -> CliResult<Option<()>> {
    try!(config.shell().set_verbosity(options.flag_verbose, options.flag_quiet));
    try!(config.shell().set_color_config(options.flag_color.as_ref().map(|s| &s[..])));

    let path = Path::new(&options.arg_path);
    let index = path.join("index");

    try!(fs::create_dir_all(&index).chain_error(|| {
        human(format!("failed to create index: `{}`", index.display()))
    }));
    let id = try!(options.flag_host.map(|s| {
        s.to_url().map(|url| SourceId::for_registry(&url)).map_err(human)
    }).unwrap_or_else(|| {
        SourceId::for_central(config)
    }));

    let lockfile = match options.flag_sync {
        Some(ref file) => file,
        None => return Ok(None),
    };

    try!(sync(Path::new(lockfile), &path, &id, config));

    println!("add this to your .cargo/config somewhere:

    [source.crates-io]
    registry = '{}'
    replace-with = 'local-registry'

    [source.local-registry]
    local-registry = '{}'

", id.url(), config.cwd().join(path).display());

    Ok(None)
}

fn sync(lockfile: &Path,
        local_dst: &Path,
        registry_id: &SourceId,
        config: &Config) -> CargoResult<()> {
    let mut registry = registry_id.load(config);
    let temp_id = SourceId::for_path(&env::current_dir().unwrap().join("tmp")).unwrap();
    let resolve = try!(cargo::ops::load_lockfile(Path::new(lockfile), &temp_id));
    let resolve = try!(resolve.chain_error(|| {
        human(format!("lock file `{}` does not exist", lockfile.display()))
    }));

    let ids = resolve.iter()
                     .filter(|id| id.source_id() == registry_id)
                     .cloned()
                     .collect::<Vec<_>>();
    for id in ids.iter() {
        let vers = format!("={}", id.version());
        let dep = try!(Dependency::parse(id.name(), Some(&vers[..]),
                                         id.source_id()));
        let vec = try!(registry.query(&dep));
        if vec.len() == 0 {
            return Err(human(format!("could not find package: {}", id)))
        }
        if vec.len() > 1 {
            return Err(human(format!("found too many packages: {}", id)))
        }

        try!(registry.download(id).chain_error(|| {
            human(format!("failed to download package from registry"))
        }));
    }

    let hash = cargo::util::hex::short_hash(registry_id);
    let ident = registry_id.url().host().unwrap().to_string();
    let part = format!("{}-{}", ident, hash);

    let index = config.registry_index_path().join(&part);
    let cache = config.registry_cache_path().join(&part);

    for id in ids.iter() {
        let filename = format!("{}-{}.crate", id.name(), id.version());
        let src = cache.join(&filename);
        let dst = local_dst.join(&filename);
        try!(fs::copy(&src, &dst).chain_error(|| {
            human(format!("failed to copy `{}` to `{}`", src.display(),
                          dst.display()))
        }));

        let name = id.name();
        let part = match name.len() {
            1 => format!("1/{}", name),
            2 => format!("2/{}", name),
            3 => format!("3/{}/{}", &name[..1], name),
            _ => format!("{}/{}/{}", &name[..2], &name[2..4], name),
        };

        let src = index.join(&part);
        let dst = local_dst.join("index").join(&part);
        try!(fs::create_dir_all(&dst.parent().unwrap()));

        let contents = try!(read(&src));

        let line = contents.lines().find(|line| {
            let pkg: RegistryPackage = rustc_serialize::json::decode(line).unwrap();
            pkg.vers == id.version().to_string()
        });
        let line = try!(line.chain_error(|| {
            human(format!("no version listed for {} in the index", id))
        }));

        let prev = read(&dst).unwrap_or(String::new());
        let mut prev = prev.lines().filter(|line| {
            let pkg: RegistryPackage = rustc_serialize::json::decode(line).unwrap();
            pkg.vers != id.version().to_string()
        }).collect::<Vec<_>>().join("\n");
        if !prev.is_empty() {
            prev.push_str("\n");
        }
        prev.push_str(&line);

        try!(File::create(&dst).and_then(|mut f| {
            f.write_all(prev.as_bytes())
        }));
    }

    Ok(())
}

fn read(path: &Path) -> CargoResult<String> {
    (|| {
        let mut contents = String::new();
        let mut f = try!(File::open(path));
        try!(f.read_to_string(&mut contents));
        Ok(contents)
    }).chain_error(|| {
        human(format!("failed to read: {}", path.display()))
    })
}
