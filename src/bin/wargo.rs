#![doc = include_str!("../../README.md")]
#![doc(
    test(attr(allow(unused_variables), deny(warnings))),
    // html_favicon_url = "https://raw.githubusercontent.com/asaaki/wargo/main/.assets/favicon.ico",
    html_logo_url = "https://raw.githubusercontent.com/asaaki/wargo/main/.assets/logo-temp.png"
)]
#![cfg_attr(feature = "docs", feature(doc_cfg))]
#![forbid(unsafe_code)]

fn main() -> wargo_lib::NullResult {
    wargo_lib::run("wargo")
}
