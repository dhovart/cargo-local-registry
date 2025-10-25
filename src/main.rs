use anyhow::Context as _;
use cargo::core::dependency::DepKind;
use cargo::core::resolver::Resolve;
use cargo::core::{Package, SourceId, Workspace};
use cargo::sources::PathSource;
use cargo::util::GlobalContext;
use cargo::util::errors::*;
use cargo_platform::Platform;
use clap::Parser as _;
use flate2::write::GzEncoder;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io;
use std::io::prelude::*;
use std::path::{self, Path, PathBuf};
use tar::{Builder, Header};
use url::Url;

use cargo_local_registry::serve_registry;

const DEFAULT_CRATE_PORT: u16 = 27283;

#[derive(Debug)]
enum FileTask {
    Copy {
        src: PathBuf,
        dst: PathBuf,
    },
    CreateArchive {
        files: Vec<PathBuf>,
        pkg_root: PathBuf,
        pkg_name: String,
        pkg_version: String,
        dst: PathBuf,
    },
}

#[derive(clap::Parser)]
#[command(version, about)]
struct Options {
    /// Registry index to sync with
    #[arg(long)]
    host: Option<String>,
    /// Vendor git dependencies as well
    #[arg(long, default_value_t = false)]
    git: bool,
    /// Use verbose output
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
    /// No output printed to stdout
    #[arg(short, long, default_value_t = false)]
    quiet: bool,
    /// Coloring: auto, always, never
    #[arg(short, long)]
    color: Option<String>,
    /// Don't delete older crates in the local registry directory
    #[arg(long)]
    no_delete: bool,

    #[command(subcommand)]
    command: SubCommands,
}

#[derive(clap::Parser)]
enum SubCommands {
    // Create a local registry
    Create {
        /// Path to Cargo.lock to sync from
        #[arg(long)]
        sync: Option<String>,

        /// Path to the local registry
        path: String,
    },

    // Serve local registry over HTTP
    Serve {
        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to bind to
        #[arg(long, default_value_t = DEFAULT_CRATE_PORT)]
        port: u16,

        /// Path to the local registry
        path: String,

        /// Disable proxying to crates.io when crates are not found locally
        #[arg(long, default_value_t = false)]
        no_proxy: bool,
    },
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
    package: Option<String>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // We're doing the vendoring operation outselves, so we don't actually want
    // to respect any of the `source` configuration in Cargo itself. That's
    // intended for other consumers of Cargo, but we want to go straight to the
    // source, e.g. crates.io, to fetch crates.
    let mut config = {
        let config_orig = GlobalContext::default().unwrap();
        let mut values = config_orig.values().unwrap().clone();
        values.remove("source");
        let config = GlobalContext::default().unwrap();
        config.set_values(values).unwrap();
        config
    };

    let options = if std::env::var("CARGO").is_err() || std::env::var("CARGO_PKG_NAME").is_ok() {
        // We're running the binary directly or inside `cargo run`.
        Options::parse()
    } else {
        // We're running as a `cargo` subcommand. Let's skip the second argument.
        let mut args = std::env::args().collect::<Vec<_>>();
        args.remove(1);
        Options::parse_from(args)
    };

    if let Err(err) = config.configure(
        options.verbose as u32,
        options.quiet,
        options.color.as_deref(),
        false,
        false,
        false,
        &None,
        &[],
        &[],
    ) {
        cargo::exit_with_error(err.into(), &mut config.shell());
    }

    let registry_url = options.host;
    let include_git = options.git;
    let remove_previously_synced = !options.no_delete;

    if let Err(err) = match options.command {
        SubCommands::Create { path, sync } => create_registry(
            path,
            sync,
            registry_url,
            include_git,
            remove_previously_synced,
            &config,
        ),
        SubCommands::Serve { host, port, path, no_proxy } => {
            serve_registry(
                host,
                port,
                path,
                registry_url,
                include_git,
                remove_previously_synced,
                !no_proxy, // Enable proxy by default, disable if no_proxy is true
                &config,
            )
            .await
        }
    } {
        cargo::exit_with_error(err.into(), &mut config.shell());
    }
}

fn create_registry(
    path: String,
    sync_lockfile: Option<String>,
    registry_url: Option<String>,
    include_git: bool,
    remove_previously_synced: bool,
    config: &GlobalContext,
) -> CargoResult<()> {
    let path = Path::new(&path);
    let index = path.join("index");

    fs::create_dir_all(&index)
        .with_context(|| format!("failed to create index: `{}`", index.display()))?;

    let id = match registry_url {
        Some(input) => SourceId::for_registry(&Url::parse(&input)?)?,
        None => SourceId::crates_io_maybe_sparse_http(config)?,
    };

    let lockfile = match sync_lockfile {
        Some(file) => file,
        None => return Ok(()),
    };

    sync(
        Path::new(&lockfile),
        path,
        &id,
        include_git,
        remove_previously_synced,
        config,
    )
    .with_context(|| "failed to sync")?;

    let registry_path = config.cwd().join(path);
    let registry_url = id.url();

    println!(
        r#"Local registry created successfully!

To use this registry, add this to your .cargo/config.toml:

    [source.crates-io]
    registry = '{}'
    replace-with = 'local-registry'

    [source.local-registry]
    local-registry = '{}'

Note: Source replacement can only be configured via config files,
not environment variables (per Cargo documentation).
"#,
        registry_url,
        registry_path.display()
    );
    Ok(())
}

fn sync(
    lockfile: &Path,
    local_dst: &Path,
    registry_id: &SourceId,
    include_git: bool,
    remove_previously_synced: bool,
    config: &GlobalContext,
) -> CargoResult<()> {
    let canonical_local_dst = local_dst.canonicalize().unwrap_or(local_dst.to_path_buf());
    let manifest = lockfile.parent().unwrap().join("Cargo.toml");
    let manifest = env::current_dir().unwrap().join(&manifest);
    let ws = Workspace::new(&manifest, config)?;
    let (packages, resolve) = cargo::ops::resolve_ws(&ws, /* dry_run */ false)
        .with_context(|| "failed to load pkg lockfile")?;
    packages.get_many(resolve.iter())?;

    let hash = cargo::util::hex::short_hash(registry_id);
    let ident = registry_id.url().host().unwrap().to_string();
    let part = format!("{}-{}", ident, hash);

    let cache = config.registry_cache_path().join(&part);

    // Phase 1: Collect all package info and file tasks (single-threaded due to Cargo API)
    let mut file_tasks = Vec::new();
    let mut package_metadata = Vec::new();

    for id in resolve.iter() {
        if id.source_id().is_git() {
            if !include_git {
                continue;
            }
        } else if !id.source_id().is_registry() {
            continue;
        }

        let pkg = packages
            .get_one(id)
            .with_context(|| "failed to fetch package")?;
        let filename = format!("{}-{}.crate", id.name(), id.version());
        let dst = canonical_local_dst.join(&filename);

        // Create file task
        if id.source_id().is_registry() {
            let src = cache.join(&filename).into_path_unlocked();
            file_tasks.push(FileTask::Copy {
                src,
                dst: dst.clone(),
            });
        } else {
            let src = PathSource::new(pkg.root(), pkg.package_id().source_id(), config);
            let files = src
                .list_files(pkg)?
                .iter()
                .map(|f| f.to_path_buf())
                .collect();
            file_tasks.push(FileTask::CreateArchive {
                files,
                pkg_root: pkg.root().to_path_buf(),
                pkg_name: pkg.name().to_string(),
                pkg_version: pkg.version().to_string(),
                dst: dst.clone(),
            });
        }

        // Store metadata for index creation
        let name = id.name().to_lowercase();
        let index_dir = canonical_local_dst.join("index");
        let index_dst = match name.len() {
            1 => index_dir.join("1").join(&name),
            2 => index_dir.join("2").join(&name),
            3 => index_dir.join("3").join(&name[..1]).join(&name),
            _ => index_dir.join(&name[..2]).join(&name[2..4]).join(&name),
        };

        package_metadata.push((
            dst,
            index_dst,
            serde_json::to_string(&registry_pkg(pkg, &resolve)).unwrap(),
            id.version().to_string(),
        ));
    }

    // Phase 2: Execute file tasks in parallel
    file_tasks
        .par_iter()
        .try_for_each(|task| -> Result<(), anyhow::Error> {
            match task {
                FileTask::Copy { src, dst } => {
                    fs::copy(src, dst).with_context(|| {
                        format!("failed to copy `{}` to `{}`", src.display(), dst.display())
                    })?;
                }
                FileTask::CreateArchive {
                    files,
                    pkg_root,
                    pkg_name,
                    pkg_version,
                    dst,
                } => {
                    let file = File::create(dst)?;
                    let gz = GzEncoder::new(file, flate2::Compression::best());
                    let mut ar = Builder::new(gz);
                    ar.mode(tar::HeaderMode::Deterministic);
                    build_ar_from_files(&mut ar, files, pkg_root, pkg_name, pkg_version)?;
                }
            }
            Ok(())
        })?;

    // Phase 3: Update index files sequentially
    let mut added_crates = HashSet::new();
    let mut added_index = HashSet::new();

    for (crate_dst, index_dst, line, version) in package_metadata {
        added_crates.insert(crate_dst);

        fs::create_dir_all(index_dst.parent().unwrap())?;

        let prev = if !remove_previously_synced || added_index.contains(&index_dst) {
            read(&index_dst).unwrap_or_default()
        } else {
            // If cleaning old entries (no_delete is not set), don't read the file unless we wrote
            // it in one of the previous iterations.
            String::new()
        };
        let mut prev_entries = prev
            .lines()
            .filter(|entry_line| {
                let pkg: RegistryPackage = serde_json::from_str(entry_line).unwrap();
                pkg.vers != version
            })
            .collect::<Vec<_>>();
        prev_entries.push(&line);
        prev_entries.sort();
        let new_contents = prev_entries.join("\n");

        File::create(&index_dst).and_then(|mut f| f.write_all(new_contents.as_bytes()))?;
        added_index.insert(index_dst);
    }

    if remove_previously_synced {
        let existing_crates: Vec<PathBuf> = canonical_local_dst
            .read_dir()
            .map(|iter| {
                iter.filter_map(|e| e.ok())
                    .filter(|e| {
                        e.file_name()
                            .to_str()
                            .is_some_and(|name| name.ends_with(".crate"))
                    })
                    .map(|e| e.path())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|_| Vec::new());

        for path in existing_crates {
            if !added_crates.contains(&path) {
                fs::remove_file(&path)?;
            }
        }

        scan_delete(&canonical_local_dst.join("index"), 3, &added_index)?;
    }
    Ok(())
}

fn scan_delete(path: &Path, depth: usize, keep: &HashSet<PathBuf>) -> CargoResult<()> {
    if path.is_file() && !keep.contains(path) {
        fs::remove_file(path)?;
    } else if path.is_dir() && depth > 0 {
        for entry in (path.read_dir()?).flatten() {
            scan_delete(&entry.path(), depth - 1, keep)?;
        }

        let is_empty = path.read_dir()?.next().is_none();
        // Don't delete "index" itself
        if is_empty && depth != 3 {
            fs::remove_dir(path)?;
        }
    }
    Ok(())
}

fn build_ar_from_files(
    ar: &mut Builder<GzEncoder<File>>,
    files: &[PathBuf],
    pkg_root: &Path,
    pkg_name: &str,
    pkg_version: &str,
) -> Result<(), anyhow::Error> {
    for file_path in files {
        let relative = file_path
            .strip_prefix(pkg_root)
            .with_context(|| format!("failed to strip prefix from {}", file_path.display()))?;
        let relative_str = relative
            .to_str()
            .with_context(|| format!("invalid unicode in path: {}", relative.display()))?;

        let mut file = File::open(file_path)
            .with_context(|| format!("failed to open file: {}", file_path.display()))?;

        let path = format!(
            "{}-{}{}{}",
            pkg_name,
            pkg_version,
            path::MAIN_SEPARATOR,
            relative_str
        );

        let mut header = Header::new_ustar();
        let metadata = file
            .metadata()
            .with_context(|| format!("failed to get metadata for: {}", file_path.display()))?;
        header
            .set_path(&path)
            .with_context(|| format!("failed to set header path: {}", path))?;
        header.set_metadata(&metadata);
        header.set_cksum();

        ar.append(&header, &mut file).with_context(|| {
            format!("failed to append file to archive: {}", file_path.display())
        })?;
    }
    Ok(())
}

fn registry_pkg(pkg: &Package, resolve: &Resolve) -> RegistryPackage {
    let id = pkg.package_id();
    let mut deps = pkg
        .dependencies()
        .iter()
        .map(|dep| {
            let (name, package) = match &dep.explicit_name_in_toml() {
                Some(explicit) => (explicit.to_string(), Some(dep.package_name().to_string())),
                None => (dep.package_name().to_string(), None),
            };

            RegistryDependency {
                name,
                req: dep.version_req().to_string(),
                features: dep.features().iter().map(|s| s.to_string()).collect(),
                optional: dep.is_optional(),
                default_features: dep.uses_default_features(),
                target: dep.platform().map(|platform| match *platform {
                    Platform::Name(ref s) => s.to_string(),
                    Platform::Cfg(ref s) => format!("cfg({})", s),
                }),
                kind: match dep.kind() {
                    DepKind::Normal => None,
                    DepKind::Development => Some("dev".to_string()),
                    DepKind::Build => Some("build".to_string()),
                },
                package,
            }
        })
        .collect::<Vec<_>>();
    deps.sort();

    let features = pkg
        .summary()
        .features()
        .iter()
        .map(|(k, v)| {
            let mut v = v.iter().map(|fv| fv.to_string()).collect::<Vec<_>>();
            v.sort();
            (k.to_string(), v)
        })
        .collect();

    RegistryPackage {
        name: id.name().to_string(),
        vers: id.version().to_string(),
        deps,
        features,
        cksum: resolve
            .checksums()
            .get(&id)
            .cloned()
            .unwrap_or_default()
            .unwrap_or_default(),
        yanked: Some(false),
    }
}

fn read(path: &Path) -> CargoResult<String> {
    let s = (|| -> io::Result<_> {
        let mut contents = String::new();
        let mut f = File::open(path)?;
        f.read_to_string(&mut contents)?;
        Ok(contents)
    })()
    .with_context(|| format!("failed to read: {}", path.display()))?;
    Ok(s)
}
