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
use tempfile::TempDir;
use url::Url;

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
    #[command(subcommand)]
    command: Option<Command>,

    /// Sync the registry with LOCK (backwards compatibility)
    #[arg(short, long)]
    sync: Option<String>,
    /// Registry index to sync with
    #[arg(long, requires = "sync")]
    host: Option<String>,
    /// Vendor git dependencies as well
    #[arg(long, default_value_t = false, requires = "sync")]
    git: bool,
    /// Don't delete older crates in the local registry directory
    #[arg(long, requires = "sync")]
    no_delete: bool,

    /// Use verbose output
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    /// No output printed to stdout
    #[arg(short, long, default_value_t = false, global = true)]
    quiet: bool,
    /// Coloring: auto, always, never
    #[arg(short, long, global = true)]
    color: Option<String>,

    /// Path to the local registry
    #[arg(global = true)]
    path: Option<String>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Sync the registry with a Cargo.lock file
    Sync {
        /// Path to Cargo.lock file
        lock: String,
        /// Registry index to sync with
        #[arg(long)]
        host: Option<String>,
        /// Vendor git dependencies as well
        #[arg(long, default_value_t = false)]
        git: bool,
        /// Don't delete older crates in the local registry directory
        #[arg(long)]
        no_delete: bool,
    },
    /// Add a crate to the registry
    Add {
        /// Name of the crate to add
        crate_name: String,
        /// Version of the crate to add (defaults to latest)
        #[arg(long)]
        version: Option<String>,
        /// Registry index to fetch from
        #[arg(long)]
        host: Option<String>,
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

fn main() {
    env_logger::init();

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
    let result = real_main(options, &mut config);
    if let Err(e) = result {
        cargo::exit_with_error(e.into(), &mut config.shell());
    }
}

fn real_main(options: Options, config: &mut GlobalContext) -> CargoResult<()> {
    config.configure(
        options.verbose as u32,
        options.quiet,
        options.color.as_deref(),
        /* frozen = */ false,
        /* locked = */ false,
        /* offline = */ false,
        /* target dir = */ &None,
        /* unstable flags = */ &[],
        /* cli_config = */ &[],
    )?;

    let path_str = options.path.as_deref().unwrap_or(".");
    let path = Path::new(path_str);
    let index = path.join("index");

    fs::create_dir_all(&index)
        .with_context(|| format!("failed to create index: `{}`", index.display()))?;

    // Handle backwards compatibility: --sync flag or sync subcommand
    if let Some(sync_path) = options.sync {
        let id = match options.host {
            Some(ref s) => SourceId::for_registry(&Url::parse(s)?)?,
            None => SourceId::crates_io_maybe_sparse_http(config)?,
        };

        sync_lockfile(
            Path::new(&sync_path),
            path,
            &id,
            options.git,
            options.no_delete,
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
    } else {
        match options.command {
            Some(Command::Sync {
                lock,
                host,
                git,
                no_delete,
            }) => {
                let id = match host {
                    Some(ref s) => SourceId::for_registry(&Url::parse(s)?)?,
                    None => SourceId::crates_io_maybe_sparse_http(config)?,
                };

                sync_lockfile(Path::new(&lock), path, &id, git, no_delete, config)
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
            }
            Some(Command::Add {
                crate_name,
                version,
                host,
            }) => {
                let id = match host {
                    Some(ref s) => SourceId::for_registry(&Url::parse(s)?)?,
                    None => SourceId::crates_io_maybe_sparse_http(config)?,
                };

                add_crate(&crate_name, version.as_deref(), path, &id, config).with_context(
                    || format!("failed to add crate `{}` with dependencies", crate_name),
                )?;

                let registry_path = config.cwd().join(path);
                config.shell().note(format!(
                    "Successfully added {} to local registry at {}",
                    crate_name,
                    registry_path.display()
                ))?;
            }
            None => {
                // No command provided and no --sync flag, just create the index directory
                return Ok(());
            }
        }
    }

    Ok(())
}

fn add_crate(
    crate_name: &str,
    version: Option<&str>,
    local_dst: &Path,
    registry_id: &SourceId,
    config: &GlobalContext,
) -> CargoResult<()> {
    // Create a temporary workspace with a Cargo.toml that depends on the requested crate
    let temp_dir = TempDir::new()?;
    let manifest_path = temp_dir.path().join("Cargo.toml");

    let dep_spec = if let Some(ver) = version {
        let version_spec = if ver.starts_with('^')
            || ver.starts_with('~')
            || ver.starts_with('=')
            || ver.starts_with('>')
            || ver.starts_with('<')
            || ver.starts_with('*')
        {
            ver.to_string()
        } else {
            format!("={}", ver)
        };
        format!("{} = \"{}\"", crate_name, version_spec)
    } else {
        format!("{} = \"*\"", crate_name)
    };

    let manifest_content = format!(
        r#"[package]
name = "temp"
version = "0.1.0"
edition = "2021"

[dependencies]
{}
"#,
        dep_spec
    );

    fs::write(&manifest_path, manifest_content)?;

    // Create a minimal src/lib.rs so Cargo doesn't complain
    let src_dir = temp_dir.path().join("src");
    fs::create_dir(&src_dir)?;
    fs::write(src_dir.join("lib.rs"), "")?;

    let original_dir = env::current_dir()?;
    env::set_current_dir(temp_dir.path())?;

    let ws = Workspace::new(&manifest_path, config)?;
    let _result = cargo::ops::resolve_ws(&ws, false)?;

    env::set_current_dir(&original_dir)?;

    let lockfile_path = temp_dir.path().join("Cargo.lock");

    sync_lockfile(&lockfile_path, local_dst, registry_id, false, true, config)?;

    Ok(())
}

fn sync_lockfile(
    lockfile: &Path,
    local_dst: &Path,
    registry_id: &SourceId,
    git: bool,
    no_delete: bool,
    config: &GlobalContext,
) -> CargoResult<()> {
    let canonical_local_dst = local_dst.canonicalize().unwrap_or(local_dst.to_path_buf());
    let manifest = lockfile.parent().unwrap().join("Cargo.toml");
    let manifest = env::current_dir().unwrap().join(&manifest);
    let ws = Workspace::new(&manifest, config)?;
    let (packages, resolve) = cargo::ops::resolve_ws(&ws, /* dry_run */ false)
        .with_context(|| "failed to load pkg lockfile")?;
    packages.get_many(resolve.iter())?;

    let cache = get_cache_path(registry_id, config);

    // Phase 1: Collect all package info and file tasks (single-threaded due to Cargo API)
    let mut file_tasks = Vec::new();
    let mut package_metadata = Vec::new();

    for id in resolve.iter() {
        if id.source_id().is_git() {
            if !git {
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
            let src = cache.join(&filename);
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
        let index_dst = get_index_path(id.name().as_str(), &canonical_local_dst);

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

        // Keep old versions if no_delete is set OR if we already updated this index file in this run
        let keep_old = no_delete || added_index.contains(&index_dst);
        update_index_entry(&index_dst, &line, &version, keep_old)?;

        added_index.insert(index_dst);
    }

    if !no_delete {
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

fn get_cache_path(registry_id: &SourceId, config: &GlobalContext) -> PathBuf {
    let hash = cargo::util::hex::short_hash(registry_id);
    let ident = registry_id.url().host().unwrap().to_string();
    let part = format!("{}-{}", ident, hash);
    config
        .registry_cache_path()
        .join(&part)
        .into_path_unlocked()
}

fn get_index_path(crate_name: &str, local_dst: &Path) -> PathBuf {
    let name = crate_name.to_lowercase();
    let index_dir = local_dst.join("index");
    match name.len() {
        1 => index_dir.join("1").join(&name),
        2 => index_dir.join("2").join(&name),
        3 => index_dir.join("3").join(&name[..1]).join(&name),
        _ => index_dir.join(&name[..2]).join(&name[2..4]).join(&name),
    }
}

fn update_index_entry(
    index_path: &Path,
    registry_package_json: &str,
    version: &str,
    keep_old_versions: bool,
) -> CargoResult<()> {
    fs::create_dir_all(index_path.parent().unwrap())?;

    let prev = if keep_old_versions {
        read(index_path).unwrap_or_default()
    } else {
        String::new()
    };

    let mut prev_entries = prev
        .lines()
        .filter(|entry_line| {
            let pkg: RegistryPackage = serde_json::from_str(entry_line).unwrap();
            pkg.vers != version
        })
        .collect::<Vec<_>>();
    prev_entries.push(registry_package_json);
    prev_entries.sort();
    let new_contents = prev_entries.join("\n");

    File::create(index_path).and_then(|mut f| f.write_all(new_contents.as_bytes()))?;
    Ok(())
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
