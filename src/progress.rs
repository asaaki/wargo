use indicatif::{ProgressBar, ProgressStyle};

pub(crate) fn bar(len: u64) -> ProgressBar {
    let style = ProgressStyle::default_bar()
        .template("{msg:.green.bold} {wide_bar:.green/blue} {pos}/{len:.bold}").expect("template issue")
        .progress_chars("█▒░");

    ProgressBar::new(len)
        .with_style(style)
        .with_message("Copying files")
}
