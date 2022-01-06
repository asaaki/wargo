#![doc = include_str!("../README.md")]
#![doc(
    test(attr(allow(unused_variables), deny(warnings))),
    // html_favicon_url = "https://raw.githubusercontent.com/asaaki/wargo/main/.assets/favicon.ico",
    html_logo_url = "https://raw.githubusercontent.com/asaaki/wargo/main/.assets/logo-temp.png"
)]
#![cfg_attr(feature = "docs", feature(doc_cfg))]
#![forbid(unsafe_code)]

use cargo_metadata::{Message, MetadataCommand};
use color_eyre::eyre::{Context, Result};
use filetime::{set_symlink_file_times, FileTime};
use globwalk::DirEntry;
use serde::Deserialize;
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    vec,
};

mod check;
mod paths;
mod progress;

type GenericResult<T> = Result<T, Box<dyn Error>>;
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

    /// internal option
    #[serde(skip)]
    clean_git: bool,
}

pub fn run(_from: &str) -> NullResult {
    color_eyre::install()?;
    check::wsl2_or_exit()?;

    let args = parse_args();

    if args.is_empty() || args[0] == "--help" {
        println!("{}", HELP_TEXT);
        return Ok(());
    }

    let workspace_root = MetadataCommand::new()
        .exec()?
        .workspace_root
        .into_std_path_buf()
        .canonicalize()?;
    let mut wargo_config = get_wargo_config(&workspace_root)?;
    let dest_dir = get_destination_dir(&wargo_config, &workspace_root);

    let entries = collect_entries(&mut wargo_config, &workspace_root)?;
    copy_files(entries, &wargo_config, &workspace_root, &dest_dir)?;

    let artifacts = exec_cargo_command(&dest_dir, &workspace_root, args)?;
    copy_artifacts(&dest_dir, &workspace_root, artifacts)?;

    Ok(())
}

fn parse_args() -> Vec<String> {
    if env::args().count() == 0 {
        return Vec::new();
    }
    let args: Vec<String> = env::args()
        .skip_while(|arg| match arg.split('/').last() {
            Some(a) => SKIPPABLES.contains(&a),
            None => false,
        })
        .collect();
    args
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
    let project_dir = if let Some(dir) = &wargo_config.project_dir {
        OsStr::new(dir)
    } else {
        workspace_root.as_ref().iter().last().unwrap()
    };
    project_dir
}

fn collect_entries<P>(
    wargo_config: &mut WargoConfig,
    workspace_root: &P,
) -> GenericResult<Vec<DirEntry>>
where
    P: AsRef<Path>,
{
    let mut patterns = vec!["**"];

    // migration phase (v0.2) - remove ignore_* blocks and de-optionize with v0.3

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
            .into_iter()
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
        fs::remove_dir_all(&dest_dir).context("dest_dir cleaning failed")?;
    }

    fs::create_dir_all(&dest_dir).context("dest_dir creation failed")?;

    let git_dir = &dest_dir.as_ref().join(".git");
    if wargo_config.clean_git && git_dir.exists() {
        fs::remove_dir_all(&git_dir).context("dest_dir/.git cleaning failed")?;
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

fn exec_cargo_command<P>(
    dest_dir: &P,
    workspace_root: &P,
    args: Vec<String>,
) -> GenericResult<Vec<PathBuf>>
where
    P: AsRef<Path>,
{
    // jump into the same relative place as it was called on the source side
    let ws_rel_location = env::current_dir()?
        .canonicalize()?
        .strip_prefix(&workspace_root)?
        .to_path_buf();
    let exec_dest = dest_dir.as_ref().join(ws_rel_location).canonicalize()?;

    let mut files: Vec<PathBuf> = Vec::new();

    let mut cargo_args = args;
    if let Some(arg) = cargo_args.first() {
        // special case: cargo build -> use JSON output
        // so we can retrieve and parse the compilation artifacts
        if arg == "build" {
            cargo_args.insert(1, "--message-format=json-render-diagnostics".into());

            let mut cmd = Command::new("cargo")
                .args(cargo_args)
                .current_dir(&exec_dest)
                .stdout(Stdio::piped())
                .spawn()?;

            let reader = std::io::BufReader::new(cmd.stdout.take().expect("no stdout captured"));
            for message in Message::parse_stream(reader) {
                if let Message::CompilerArtifact(artifact) = message.unwrap() {
                    if artifact.target.kind[0] == "bin" {
                        for filename in artifact.filenames {
                            files.push(filename.into_std_path_buf())
                        }
                    }
                }
            }
            cmd.wait()?;
        } else {
            let mut cmd = Command::new("cargo")
                .args(cargo_args)
                .current_dir(&exec_dest)
                .spawn()?;
            cmd.wait()?;
        }
    };
    Ok(files)
}

fn copy_artifacts<P>(dest_dir: &P, workspace_root: &P, artifacts: Vec<PathBuf>) -> NullResult
where
    P: AsRef<Path>,
{
    if !artifacts.is_empty() {
        for artifact in artifacts {
            let rel_artifact = artifact.strip_prefix(&dest_dir)?;
            let origin_location = &workspace_root.as_ref().join(rel_artifact);

            if let Some(parent) = origin_location.parent() {
                fs::create_dir_all(&parent)?;
                fs::copy(artifact, origin_location)?;
                eprintln!("Copied compile artifact to: {}", origin_location.display());
            }
        }
    };
    Ok(())
}
