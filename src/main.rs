extern crate cargo;
extern crate flate2;
extern crate rustc_serialize;
extern crate tar;

use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::{self, Path};

use cargo::CliResult;
use cargo::core::{SourceId, Dependency, Workspace, Package};
use cargo::core::dependency::{Kind, Platform};
use cargo::sources::PathSource;
use cargo::util::{human, ChainError, ToUrl, Config, CargoResult};
use flate2::write::GzEncoder;
use rustc_serialize::json;
use tar::{Builder, Header};

#[derive(RustcDecodable)]
struct Options {
    arg_path: String,
    flag_sync: Option<String>,
    flag_host: Option<String>,
    flag_verbose: u32,
    flag_quiet: Option<bool>,
    flag_color: Option<String>,
    flag_git: bool,
}

#[derive(RustcDecodable, RustcEncodable)]
struct RegistryPackage {
    name: String,
    vers: String,
    deps: Vec<RegistryDependency>,
    features: HashMap<String, Vec<String>>,
    cksum: String,
    yanked: Option<bool>,
}

#[derive(RustcDecodable, RustcEncodable)]
struct RegistryDependency {
    name: String,
    req: String,
    features: Vec<String>,
    optional: bool,
    default_features: bool,
    target: Option<String>,
    kind: Option<String>,
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
    --git                    Vendor git dependencies as well
    -v, --verbose            Use verbose output
    -q, --quiet              No output printed to stdout
    --color WHEN             Coloring: auto, always, never
"#)
}

fn real_main(options: Options, config: &Config) -> CliResult<Option<()>> {
    try!(config.configure(options.flag_verbose,
                          options.flag_quiet,
                          &options.flag_color,
                          /* frozen = */ false,
                          /* locked = */ false));

    let path = Path::new(&options.arg_path);
    let index = path.join("index");

    try!(fs::create_dir_all(&index).chain_error(|| {
        human(format!("failed to create index: `{}`", index.display()))
    }));
    let id = try!(options.flag_host.map(|s| {
        s.to_url().map(|url| SourceId::for_registry(&url)).map_err(human)
    }).unwrap_or_else(|| {
        SourceId::crates_io(config)
    }));

    let lockfile = match options.flag_sync {
        Some(ref file) => file,
        None => return Ok(None),
    };

    try!(sync(Path::new(lockfile), &path, &id, config).chain_error(|| {
        human("failed to sync")
    }));

    if options.flag_git {
        try!(sync_git(Path::new(lockfile), &path, config).chain_error(|| {
            human("failed to sync git repos")
        }));
    }

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
    let manifest = lockfile.parent().unwrap().join("Cargo.toml");
    let manifest = env::current_dir().unwrap().join(&manifest);
    let ws = try!(Workspace::new(&manifest, config));
    let resolve = try!(cargo::ops::load_pkg_lockfile(&ws).chain_error(|| {
        human("failed to load pkg lockfile")
    }));
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
        let src = cache.join(&filename).into_path_unlocked();
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

        let src = index.join(&part).into_path_unlocked();
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

fn sync_git(lockfile: &Path,
            local_dst: &Path,
            config: &Config) -> CargoResult<()> {
    let manifest = lockfile.parent().unwrap().join("Cargo.toml");
    let manifest = env::current_dir().unwrap().join(&manifest);
    let ws = try!(Workspace::new(&manifest, config));
    let resolve = try!(cargo::ops::load_pkg_lockfile(&ws).chain_error(|| {
        human("failed to load pkg lockfile")
    }));
    let resolve = try!(resolve.chain_error(|| {
        human(format!("lock file `{}` does not exist", lockfile.display()))
    }));

    let ids = resolve.iter()
                     .filter(|id| id.source_id().is_git())
                     .collect::<Vec<_>>();
    for id in ids.iter() {
        let any_registry = resolve.iter()
                                  .filter(|p| p.name() == id.name())
                                  .any(|p| !p.source_id().is_git());
        if any_registry {
            panic!("git dependency shares names with other dep: {}", id.name());
        }
        let dep = try!(Dependency::parse(id.name(), None, id.source_id()));
        let mut source = id.source_id().load(config);
        try!(source.update());
        let vec = try!(source.query(&dep));
        if vec.len() == 0 {
            return Err(human(format!("could not find package: {}", id)))
        }
        if vec.len() > 1 {
            return Err(human(format!("found too many packages: {}", id)))
        }
        let pkg = try!(source.download(id).chain_error(|| {
            human(format!("failed to download package from registry"))
        }));

        let filename = format!("{}-{}.crate", id.name(), id.version());
        let dst = local_dst.join(&filename);
        let file = File::create(&dst).unwrap();
        let gz = GzEncoder::new(file, flate2::Compression::Best);
        let mut ar = Builder::new(gz);
        build_ar(&mut ar, &pkg, config);

        let name = id.name();
        let part = match name.len() {
            1 => format!("1/{}", name),
            2 => format!("2/{}", name),
            3 => format!("3/{}/{}", &name[..1], name),
            _ => format!("{}/{}/{}", &name[..2], &name[2..4], name),
        };

        let dst = local_dst.join("index").join(&part);
        assert!(!dst.exists());
        try!(fs::create_dir_all(&dst.parent().unwrap()));

        let pkg = RegistryPackage {
            name: id.name().to_string(),
            vers: id.version().to_string(),
            deps: pkg.dependencies().iter().map(|dep| {
                RegistryDependency {
                    name: dep.name().to_string(),
                    req: dep.version_req().to_string(),
                    features: dep.features().to_owned(),
                    optional: dep.is_optional(),
                    default_features: dep.uses_default_features(),
                    target: dep.platform().map(|platform| {
                        match *platform {
                            Platform::Name(ref s) => s.to_string(),
                            Platform::Cfg(ref s) => format!("cfg({})", s),
                        }
                    }),
                    kind: match dep.kind() {
                        Kind::Normal => None,
                        Kind::Development => Some("dev".to_string()),
                        Kind::Build => Some("build".to_string()),
                    },
                }
            }).collect(),
            features: pkg.summary().features().clone(),
            cksum: String::new(),
            yanked: None,
        };
        let line = json::encode(&pkg).unwrap();

        try!(File::create(&dst).and_then(|mut f| {
            f.write_all(line.as_bytes())
        }));
    }

    return Ok(());

    fn build_ar(ar: &mut Builder<GzEncoder<File>>,
                pkg: &Package,
                config: &Config) {
        let root = pkg.root();
        let src = PathSource::new(pkg.root(),
                                  pkg.package_id().source_id(),
                                  config);
        for file in src.list_files(pkg).unwrap().iter() {
            let relative = cargo::util::without_prefix(&file, &root).unwrap();
            let relative = relative.to_str().unwrap();
            let mut file = File::open(file).unwrap();
            let path = format!("{}-{}{}{}", pkg.name(), pkg.version(),
                               path::MAIN_SEPARATOR, relative);

            let mut header = Header::new_ustar();
            let metadata = file.metadata().unwrap();
            header.set_path(&path).unwrap();
            header.set_metadata(&metadata);
            header.set_cksum();

            ar.append(&header, &mut file).unwrap();
        }
    }
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
