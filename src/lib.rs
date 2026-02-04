#![doc = include_str!("../README.md")]
#![doc(
    test(attr(allow(unused_variables), deny(warnings))),
    // html_favicon_url = "https://raw.githubusercontent.com/asaaki/wargo/main/.assets/favicon.ico",
    html_logo_url = "https://raw.githubusercontent.com/asaaki/wargo/main/.assets/logo-temp.png"
)]
#![cfg_attr(feature = "docs", feature(doc_cfg))]
#![forbid(unsafe_code)]

use anyhow::Context;
use cargo_metadata::{Message, MetadataCommand, TargetKind};
use cprint::cprintln;
use filetime::{FileTime, set_symlink_file_times};
use globwalk::DirEntry;
use serde::Deserialize;
use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    vec,
};

mod check;
mod paths;
mod progress;

type GenericResult<T> = anyhow::Result<T>;
pub type NullResult = GenericResult<()>;

const SKIPPABLES: [&str; 4] = ["wargo", "cargo-wsl", "cargo", "wsl"];

const HELP_TEXT: &str = r#"wargo

cargo's evil twin to work with projects in the twilight zone of WSL2

HELP TEXT WENT MISSING IN THE DARK …

Maybe you find more helpful information at:

https://github.com/asaaki/wargo
"#;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct WargoConfig {
    /// optionally override the project folder name
    /// in the destination base directory
    project_dir: Option<String>,

    /// optionally set a different destination base directory
    dest_base_dir: Option<String>,

    /// @deprecated - will be removed in v0.3
    ignore_git: Option<bool>,

    /// preferred since v0.2
    /// de-Option with v0.3
    include_git: Option<bool>,

    /// @deprecated - will be removed in v0.3
    ignore_target: Option<bool>,

    /// preferred since v0.2
    /// de-Option with v0.3
    include_target: Option<bool>,

    /// clean out the project folder before run
    /// (will remove and recreate folder)
    clean: bool,

    /// optional run cwd override for `wargo run`
    run_cwd: Option<String>,

    /// internal option
    #[serde(skip)]
    clean_git: bool,
}

pub fn run(_from: &str) -> NullResult {
    #[cfg(target_os = "windows")]
    if wsl2_subshell()? {
        cprintln!("wargo", "WSL2 subshell done." => Cyan);
        return Ok(());
    }
    check::wsl2_or_exit()?;

    let args = parse_args();

    if args.is_empty() || args[0] == "--help" {
        println!("{HELP_TEXT}");
        return Ok(());
    }

    let workspace_root = MetadataCommand::new()
        .exec()?
        .workspace_root
        .into_std_path_buf()
        .canonicalize()?;
    let mut wargo_config = get_wargo_config(&workspace_root)?;
    let dest_dir = get_destination_dir(&wargo_config, &workspace_root);

    let (cargo_args, cli_run_cwd) = extract_run_cwd(args)?;
    let (mut run_cwd, run_cwd_source) = if let Some(cli_run_cwd) = cli_run_cwd {
        let base_dir = env::current_dir()?;
        (
            resolve_run_cwd_with_base(Some(cli_run_cwd), &base_dir, "--run-cwd")?,
            Some("cli"),
        )
    } else if let Some(config_run_cwd) = wargo_config.run_cwd.as_ref().map(PathBuf::from) {
        (
            resolve_run_cwd_with_base(Some(config_run_cwd), &workspace_root, "Wargo.toml run_cwd")?,
            Some("config"),
        )
    } else {
        (None, None)
    };

    if run_cwd.is_some() {
        let is_run = cargo_args
            .first()
            .map(|arg| ["r", "run"].contains(&arg.as_str()))
            .unwrap_or(false);
        if !is_run {
            if run_cwd_source == Some("cli") {
                return Err(anyhow::anyhow!("--run-cwd can only be used with `run`"));
            }
            run_cwd = None;
        } else {
            let has_manifest_path = cargo_args
                .iter()
                .any(|arg| arg == "--manifest-path" || arg.starts_with("--manifest-path="));
            if has_manifest_path {
                return Err(anyhow::anyhow!(
                    "--run-cwd cannot be combined with cargo's --manifest-path"
                ));
            }
        }
    }

    let entries = collect_entries(&mut wargo_config, &workspace_root)?;
    copy_files(entries, &wargo_config, &workspace_root, &dest_dir)?;

    let (artifacts, exit_code) =
        exec_cargo_command(&dest_dir, &workspace_root, cargo_args, run_cwd)?;
    copy_artifacts(&dest_dir, &workspace_root, artifacts)?;

    if let Some(code) = exit_code
        && code != 0
    {
        std::process::exit(code);
    }

    Ok(())
}

// TODO: add WSL distro configuration and use it here
// TODO: add option to use a different shell (e.g. zsh)
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn wsl2_subshell() -> GenericResult<bool> {
    if !check::is_wsl2() {
        let wargo_args = parse_args()[1..].join(" ");
        let wargo_and_args = format!("wargo {wargo_args}");
        let args = ["--shell-type", "login", "--", "bash", "-c", &wargo_and_args];

        cprintln!("wargo", "WSL2 subshelling ..." => Cyan);
        Command::new("wsl")
            .env("WARGO_RUN", "1")
            .args(args)
            .spawn()?
            .wait()?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn parse_args() -> Vec<String> {
    if env::args().count() == 0 {
        return Vec::new();
    }
    let args: Vec<String> = env::args()
        .skip_while(|arg| match arg.split('/').next_back() {
            Some(a) => SKIPPABLES.contains(&a),
            None => false,
        })
        .collect();
    args
}

fn extract_run_cwd(args: Vec<String>) -> GenericResult<(Vec<String>, Option<PathBuf>)> {
    let mut run_cwd: Option<PathBuf> = None;
    let mut filtered: Vec<String> = Vec::with_capacity(args.len());

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--run-cwd" {
            let value = iter.next().context("--run-cwd expects a directory path")?;
            run_cwd = Some(PathBuf::from(value));
            continue;
        }

        if let Some(value) = arg.strip_prefix("--run-cwd=") {
            if value.is_empty() {
                return Err(anyhow::anyhow!("--run-cwd expects a directory path"));
            }
            run_cwd = Some(PathBuf::from(value));
            continue;
        }

        filtered.push(arg);
    }

    Ok((filtered, run_cwd))
}

fn resolve_run_cwd_with_base(
    run_cwd: Option<PathBuf>,
    base_dir: &Path,
    source: &str,
) -> GenericResult<Option<PathBuf>> {
    let Some(run_cwd) = run_cwd else {
        return Ok(None);
    };

    let mut resolved = if run_cwd.is_absolute() {
        run_cwd
    } else {
        base_dir.join(run_cwd)
    };

    resolved = resolved
        .canonicalize()
        .with_context(|| format!("run cwd does not exist: {}", resolved.display()))?;

    if !resolved.is_dir() {
        return Err(anyhow::anyhow!(
            "{source} must point to an existing directory"
        ));
    }

    Ok(Some(paths::normalize_path(&resolved)))
}

fn get_wargo_config<P>(workspace_root: &P) -> GenericResult<WargoConfig>
where
    P: AsRef<Path>,
{
    let wargo_config = workspace_root.as_ref().join("Wargo.toml");

    let wargo_config: WargoConfig = if wargo_config.exists() {
        let wargo_config = fs::read_to_string(wargo_config)?;
        toml::from_str(&wargo_config)?
    } else {
        WargoConfig::default()
    };

    Ok(wargo_config)
}

fn get_destination_dir<P>(wargo_config: &WargoConfig, workspace_root: &P) -> PathBuf
where
    P: AsRef<Path>,
{
    let project_dir = get_project_dir(wargo_config, &workspace_root);

    let dest = if let Some(dir) = &wargo_config.dest_base_dir {
        paths::untilde(dir)
    } else {
        // TODO(maybe): handle rare case of None (if no home dir can be determined)
        let home = dirs::home_dir().unwrap();
        home.join("tmp")
    }
    .join(project_dir);

    paths::normalize_path(&dest)
}

fn get_project_dir<'a, P>(wargo_config: &'a WargoConfig, workspace_root: &'a P) -> &'a OsStr
where
    P: AsRef<Path>,
{
    (if let Some(dir) = &wargo_config.project_dir {
        OsStr::new(dir)
    } else {
        workspace_root.as_ref().iter().next_back().unwrap()
    }) as _
}

fn collect_entries<P>(
    wargo_config: &mut WargoConfig,
    workspace_root: &P,
) -> GenericResult<Vec<DirEntry>>
where
    P: AsRef<Path>,
{
    let mut patterns = vec!["**"];

    // migration phase (v0.2) - remove ignore_* blocks and de-optionize with v0.4 or later

    if let Some(include_git) = wargo_config.include_git {
        if !include_git {
            patterns.push("!.git");
        } else {
            wargo_config.clean_git = true;
        }
    } else if let Some(ignore_git) = wargo_config.ignore_git {
        if ignore_git {
            patterns.push("!.git");
        } else {
            wargo_config.clean_git = true;
        }
    } else {
        // default if no option was provided
        patterns.push("!.git");
    }

    if let Some(include_target) = wargo_config.include_target {
        if !include_target {
            patterns.push("!target");
        }
    } else if let Some(ignore_target) = wargo_config.ignore_target {
        if ignore_target {
            patterns.push("!target");
        }
    } else {
        // default if no option was provided
        patterns.push("!target");
    }

    let entries: Vec<DirEntry> =
        globwalk::GlobWalkerBuilder::from_patterns(workspace_root, &patterns)
            .contents_first(false)
            .build()?
            .filter_map(Result::ok)
            .collect();
    Ok(entries)
}

fn copy_files<P>(
    entries: Vec<DirEntry>,
    wargo_config: &WargoConfig,
    workspace_root: &P,
    dest_dir: &P,
) -> NullResult
where
    P: AsRef<Path>,
{
    if wargo_config.clean && dest_dir.as_ref().exists() {
        fs::remove_dir_all(dest_dir).context("dest_dir cleaning failed")?;
    }

    fs::create_dir_all(dest_dir).context("dest_dir creation failed")?;

    let git_dir = &dest_dir.as_ref().join(".git");
    if wargo_config.clean_git && git_dir.exists() {
        fs::remove_dir_all(git_dir).context("dest_dir/.git cleaning failed")?;
    }

    let bar = progress::bar(entries.len() as u64);
    for entry in bar.wrap_iter(entries.iter()) {
        let is_dir = entry.file_type().is_dir();
        let src_path = entry.path();
        let prj_path = src_path.strip_prefix(workspace_root)?;
        let dst_path = &dest_dir.as_ref().to_path_buf().join(prj_path);

        let metadata = entry.metadata()?;
        let mtime = FileTime::from_last_modification_time(&metadata);
        let atime = FileTime::from_last_access_time(&metadata);

        if is_dir {
            fs::create_dir_all(dst_path).context("Directory creation failed")?;
        } else {
            // TODO(maybe): should skip if file is unchanged;
            // OTOH it would mean more FS calls/checks
            fs::copy(src_path, dst_path).with_context(|| {
                format!(
                    "Copying failed: {} -> {}",
                    &src_path.display(),
                    &dst_path.display()
                )
            })?;
        }

        set_symlink_file_times(dst_path, atime, mtime).with_context(|| {
            format!("Setting file timestamps failed for {}", &dst_path.display())
        })?;
    }
    bar.finish_with_message("Files copied");
    Ok(())
}

fn find_manifest_path(start: &Path, stop_at: &Path) -> Option<PathBuf> {
    let mut current = start;
    loop {
        let candidate = current.join("Cargo.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        if current == stop_at {
            return None;
        }
        current = current.parent()?;
    }
}

fn exec_cargo_command<P>(
    dest_dir: &P,
    workspace_root: &P,
    args: Vec<String>,
    run_cwd: Option<PathBuf>,
) -> GenericResult<(Vec<PathBuf>, Option<i32>)>
where
    P: AsRef<Path>,
{
    // jump into the same relative place as it was called on the source side
    let ws_rel_location = env::current_dir()?
        .canonicalize()?
        .strip_prefix(workspace_root)?
        .to_path_buf();
    let exec_dest = dest_dir.as_ref().join(ws_rel_location).canonicalize()?;

    let mut files: Vec<PathBuf> = Vec::new();
    let mut exit_code = None;

    let mut cargo_args = args;
    if let Some(arg) = cargo_args.first() {
        // special case: cargo build -> use JSON output
        // so we can retrieve and parse the compilation artifacts
        if ["b", "build"].contains(&arg.as_str()) {
            cargo_args.insert(1, "--message-format=json-render-diagnostics".into());

            let mut cmd = Command::new("cargo")
                .args(cargo_args)
                .current_dir(&exec_dest)
                .stdout(Stdio::piped())
                .spawn()?;

            let reader = std::io::BufReader::new(cmd.stdout.take().expect("no stdout captured"));
            for message in Message::parse_stream(reader).flatten() {
                if let Message::CompilerArtifact(artifact) = message
                    && [
                        TargetKind::Bin,
                        TargetKind::DyLib,
                        TargetKind::CDyLib,
                        TargetKind::StaticLib,
                    ]
                    .contains(&artifact.target.kind[0])
                {
                    for filename in artifact.filenames {
                        files.push(filename.into_std_path_buf())
                    }
                }
            }
            let status = cmd.wait()?;
            exit_code = status.code();
        } else if ["r", "run"].contains(&arg.as_str())
            && let Some(run_cwd) = run_cwd
        {
            let manifest_path = find_manifest_path(&exec_dest, dest_dir.as_ref())
                .context("Cargo.toml not found when trying to use --run-cwd")?;
            cargo_args.insert(1, "--manifest-path".into());
            cargo_args.insert(2, manifest_path.to_string_lossy().into());

            let mut cmd = Command::new("cargo")
                .args(cargo_args)
                .current_dir(run_cwd)
                .spawn()?;
            let status = cmd.wait()?;
            exit_code = status.code();
        } else {
            let mut cmd = Command::new("cargo")
                .args(cargo_args)
                .current_dir(&exec_dest)
                .spawn()?;
            let status = cmd.wait()?;
            exit_code = status.code();
        }
    };
    Ok((files, exit_code))
}

fn copy_artifacts<P>(dest_dir: &P, workspace_root: &P, artifacts: Vec<PathBuf>) -> NullResult
where
    P: AsRef<Path>,
{
    if !artifacts.is_empty() {
        for artifact in artifacts {
            let rel_artifact = artifact.strip_prefix(dest_dir)?;
            let origin_location = &workspace_root.as_ref().join(rel_artifact);

            if let Some(parent) = origin_location.parent() {
                fs::create_dir_all(parent)?;
                fs::copy(artifact, origin_location)?;
                cprintln!(
                    "Copied",
                    format!("compile artifact to:\n{}", origin_location.display()) => Green
                );
            }
        }
    };
    Ok(())
}
