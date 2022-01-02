use indicatif::{ProgressBar, ProgressStyle};

pub(crate) fn bar(len: u64) -> ProgressBar {
    let style = ProgressStyle::default_bar()
        .template(
            //  [{eta_precise} / {elapsed_precise:.cyan}]
            "{msg:.green.bold} {wide_bar:.green/blue} {pos}/{len:.bold}",
        )
        .progress_chars("█▒░");

    ProgressBar::new(len)
        .with_style(style)
        .with_message("Copying files")
}
