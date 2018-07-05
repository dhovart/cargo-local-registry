extern crate cargo;
extern crate docopt;
extern crate env_logger;
extern crate failure;
extern crate flate2;
extern crate tar;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use std::io;
use std::path::{self, Path};

use cargo::core::{SourceId, Workspace, Package};
use cargo::core::dependency::{Kind, Platform};
use cargo::sources::PathSource;
use cargo::util::{ToUrl, Config};
use cargo::util::errors::*;
use docopt::Docopt;
use flate2::write::GzEncoder;
use tar::{Builder, Header};

#[derive(Deserialize)]
struct Options {
    arg_path: String,
    flag_sync: Option<String>,
    flag_host: Option<String>,
    flag_verbose: u32,
    flag_quiet: Option<bool>,
    flag_color: Option<String>,
    flag_git: bool,
}

#[derive(Deserialize, Serialize)]
struct RegistryPackage {
    name: String,
    vers: String,
    deps: Vec<RegistryDependency>,
    cksum: String,
    features: BTreeMap<String, Vec<String>>,
    yanked: Option<bool>,
}

#[derive(Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
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
    env_logger::init();
    let mut config = Config::default().unwrap();
    let usage = r#"
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
"#;

    let options = Docopt::new(usage)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());
    let result = real_main(options, &mut config);
    if let Err(e) = result {
        cargo::exit_with_error(e.into(), &mut *config.shell());
    }
}

fn real_main(options: Options, config: &mut Config) -> CargoResult<()> {
    try!(config.configure(options.flag_verbose,
                          options.flag_quiet,
                          &options.flag_color,
                          /* frozen = */ false,
                          /* locked = */ false,
                          /* target dir = */ &None,
                          /* unstable flags = */ &[]));

    let path = Path::new(&options.arg_path);
    let index = path.join("index");

    try!(fs::create_dir_all(&index).chain_err(|| {
        format!("failed to create index: `{}`", index.display())
    }));
    let id = match options.flag_host {
        Some(ref s) => SourceId::for_registry(&s.to_url()?)?,
        None => SourceId::crates_io(config)?,
    };

    let lockfile = match options.flag_sync {
        Some(ref file) => file,
        None => return Ok(()),
    };

    sync(Path::new(lockfile), &path, &id, &options, config).chain_err(|| {
        "failed to sync"
    })?;

    println!("add this to your .cargo/config somewhere:

    [source.crates-io]
    registry = '{}'
    replace-with = 'local-registry'

    [source.local-registry]
    local-registry = '{}'

", id.url(), config.cwd().join(path).display());

    Ok(())
}

fn sync(lockfile: &Path,
        local_dst: &Path,
        registry_id: &SourceId,
        options: &Options,
        config: &Config) -> CargoResult<()> {
    let manifest = lockfile.parent().unwrap().join("Cargo.toml");
    let manifest = env::current_dir().unwrap().join(&manifest);
    let ws = Workspace::new(&manifest, config)?;
    let (packages, resolve) = cargo::ops::resolve_ws(&ws).chain_err(|| {
        "failed to load pkg lockfile"
    })?;
    let hash = cargo::util::hex::short_hash(registry_id);
    let ident = registry_id.url().host().unwrap().to_string();
    let part = format!("{}-{}", ident, hash);

    let cache = config.registry_cache_path().join(&part);

    for id in resolve.iter() {
        if id.source_id().is_git() {
            if !options.flag_git {
                continue
            }
        } else if !id.source_id().is_registry() {
            continue
        }

        let pkg = packages.get(&id).chain_err(|| "failed to fetch package")?;
        let filename = format!("{}-{}.crate", id.name(), id.version());
        let dst = local_dst.join(&filename);
        if id.source_id().is_registry() {
            let src = cache.join(&filename).into_path_unlocked();
            fs::copy(&src, &dst).chain_err(|| {
                format!("failed to copy `{}` to `{}`", src.display(),
                        dst.display())
            })?;
        } else {
            let file = File::create(&dst).unwrap();
            let gz = GzEncoder::new(file, flate2::Compression::best());
            let mut ar = Builder::new(gz);
            ar.mode(tar::HeaderMode::Deterministic);
            build_ar(&mut ar, &pkg, config);
        }

        let name = id.name();
        let part = match name.len() {
            1 => format!("1/{}", name),
            2 => format!("2/{}", name),
            3 => format!("3/{}/{}", &name[..1], name),
            _ => format!("{}/{}/{}", &name[..2], &name[2..4], name),
        };

        let dst = local_dst.join("index").join(&part);
        fs::create_dir_all(&dst.parent().unwrap())?;
        let line = serde_json::to_string(&registry_pkg(&pkg)).unwrap();

        let prev = read(&dst).unwrap_or(String::new());
        let mut prev_entries = prev.lines().filter(|line| {
            let pkg: RegistryPackage = serde_json::from_str(line).unwrap();
            pkg.vers != id.version().to_string()
        }).collect::<Vec<_>>();
        prev_entries.push(&line);
        prev_entries.sort();
        let new_contents = prev_entries.join("\n");

        File::create(&dst).and_then(|mut f| {
            f.write_all(new_contents.as_bytes())
        })?;
    }

    Ok(())
}

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

fn registry_pkg(pkg: &Package) -> RegistryPackage {
    let id = pkg.package_id();
    let mut deps = pkg.dependencies().iter().map(|dep| {
        RegistryDependency {
            name: dep.name().to_string(),
            req: dep.version_req().to_string(),
            features: dep.features().iter().map(|s| s.to_string()).collect(),
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
    }).collect::<Vec<_>>();
    deps.sort();

    let features = pkg.summary()
                      .features()
                      .into_iter()
                      .map(|(k, v)| {
                          let mut v = v.iter()
                              .map(|x| x.to_string(pkg.summary()))
                              .collect::<Vec<_>>();
                          v.sort();
                          (k.clone(), v)
                      })
                      .collect();

    RegistryPackage {
        name: id.name().to_string(),
        vers: id.version().to_string(),
        deps: deps,
        features: features,
        cksum: pkg.summary().checksum().map(|s| s.to_string()).unwrap_or_default(),
        yanked: Some(false),
    }
}

fn read(path: &Path) -> CargoResult<String> {
    let s = (|| -> io::Result<_> {
        let mut contents = String::new();
        let mut f = File::open(path)?;
        f.read_to_string(&mut contents)?;
        Ok(contents)
    })().chain_err(|| {
        format!("failed to read: {}", path.display())
    })?;
    Ok(s)
}
