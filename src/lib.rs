use cargo_metadata::{Message, MetadataCommand};
use filetime::{set_symlink_file_times, FileTime};
use globwalk::DirEntry;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    process::{Command, Stdio},
    result::Result,
    vec,
};

mod check;
mod paths;

type GenericResult<T> = Result<T, Box<dyn Error>>;
pub type NullResult = GenericResult<()>;
// type IoResult = std::io::Result<()>;
// type OkResult<T> = Result<T, core::convert::Infallible>;
// type StringResult = OkResult<String>;

// cargo run --bin wargo -- build --message-format=json-render-diagnostics
// cargo run --bin wargo -- build --message-format=json-diagnostic-short
// cargo run --bin wargo -- build --message-format=short
// cargo run --bin wargo -- build
// cargo install --path . --bins
// wargo build

const SKIPPABLES: [&str; 4] = ["wargo", "cargo-wsl", "cargo", "wsl"];

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct WargoConfig<'a> {
    project_dir: Option<&'a str>,
    dest_base_dir: Option<&'a str>,
    use_mktemp: bool,
    mktemp_rchars: u8,
    #[serde(default = "default_true")]
    ignore_git: bool,
    #[serde(default = "default_true")]
    ignore_target: bool,
    clean: bool,
}

fn default_true() -> bool {
    true
}

pub fn run(_from: &str) -> NullResult {
    // dbg!(_from);

    if check::is_not_wsl2() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "This command can only be used in a WSL2 environment",
        )
        .into());
    }

    let args: Vec<String> = env::args()
        .skip_while(|arg| match arg.split('/').last() {
            Some(a) => SKIPPABLES.contains(&a),
            None => false,
        })
        .collect();

    if args[0] == "--help" {
        println!(r#"wargo

cargo's evil twin to work with projects in the twilight zone of WSL2

HELP TEXT WENT MISSING IN THE DARK …
        "#);
        return Ok(())
    }

    let cmd = MetadataCommand::new();
    let metadata = cmd.exec()?;

    let current_path = env::current_dir()?.canonicalize()?;
    let workspace_root = metadata.workspace_root.into_std_path_buf().canonicalize()?;
    let ws_rel_location = current_path.strip_prefix(&workspace_root)?;

    let wargo_config = workspace_root.join("Wargo.toml");
    let wargo_config = fs::read_to_string(wargo_config)?;
    let wargo_config: WargoConfig = toml::from_str(&wargo_config)?;

    let project_dir = if let Some(dir) = wargo_config.project_dir {
        &OsStr::new(dir)
    } else {
        workspace_root.iter().last().unwrap()
    };

    let dest = if let Some(dir) = wargo_config.dest_base_dir {
        paths::untilde(dir)
    } else {
        // TODO(maybe): handle rare case of None (if no home dir can be determined)
        let home = dirs::home_dir().unwrap();
        home.join("tmp")
    }
    .join(project_dir);
    let dest = paths::normalize_path(&dest);

    // TODO(maybe): diff source and target; add new, remove old

    let mut patterns = vec!["**"];
    if wargo_config.ignore_git {
        patterns.push("!.git")
    }
    if wargo_config.ignore_target {
        patterns.push("!target")
    }

    let entries: Vec<DirEntry> =
        globwalk::GlobWalkerBuilder::from_patterns(&workspace_root, &patterns)
            .contents_first(false)
            .build()?
            .into_iter()
            .filter_map(Result::ok)
            .collect();

    if wargo_config.clean && dest.exists() {
        fs::remove_dir_all(&dest)?;
    }

    fs::create_dir_all(&dest)?;

    let bar = bar(entries.len() as u64);
    for entry in bar.wrap_iter(entries.iter()) {
        let is_dir = entry.file_type().is_dir();
        let sp = entry.path();
        let pp = sp.strip_prefix(&workspace_root)?;
        let dp = &dest.to_path_buf().join(pp);

        let mt = entry.metadata()?;
        let mtime = FileTime::from_last_modification_time(&mt);
        let atime = FileTime::from_last_access_time(&mt);

        if is_dir {
            fs::create_dir_all(dp)?;
        } else {
            // TODO(maybe): should skip if file is unchanged;
            // OTOH it would mean more FS calls/checks
            fs::copy(sp, dp)?;
        }

        set_symlink_file_times(dp, atime, mtime)?;
    }
    bar.finish_with_message("Files copied");

    // jump into the same relative place as it was called on the source side
    let exec_dest = dest.join(ws_rel_location).canonicalize()?;
    let mut cargo_args = args;

    if let Some(arg) = cargo_args.first() {
        if arg == "build" {
            cargo_args.insert(1, "--message-format=json-render-diagnostics".into());

            let mut cmd = Command::new("cargo")
                .args(cargo_args)
                .current_dir(&exec_dest)
                .stdout(Stdio::piped())
                .spawn()?;

            let mut files = Vec::new();

            let reader = std::io::BufReader::new(cmd.stdout.take().expect("no stdout captured"));
            for message in Message::parse_stream(reader) {
                match message.unwrap() {
                    Message::CompilerArtifact(artifact) => {
                        if artifact.target.kind[0] == "bin" {
                            files.extend_from_slice(&artifact.filenames);
                        }
                    }
                    _ => (),
                }
            }
            cmd.wait()?;
            if files.len() > 0 {
                eprintln!("final files created:\n{:?}", files);
            }
        } else {
            let mut cmd = Command::new("cargo")
                .args(cargo_args)
                .current_dir(&exec_dest)
                .spawn()?;
            cmd.wait()?;
        }
    }

    // TODO(maybe): copy artefacts back to source location

    Ok(())
}

fn bar(len: u64) -> ProgressBar {
    let style = ProgressStyle::default_bar()
        .template(
            //  [{eta_precise} / {elapsed_precise:.cyan}]
            "{msg:.green.bold} {wide_bar:.green/blue} {pos}/{len:.bold}",
        )
        .progress_chars("█▒░");
    let bar = ProgressBar::new(len)
        .with_style(style)
        .with_message("Copying files");
    bar
}
