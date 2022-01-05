<h1 align="center" title="wargo"><img src="https://raw.githubusercontent.com/asaaki/wargo/main/.assets/logo-temp@2x.png" width=128 height=128 title="wargo"></h1>
<div align="center">

_cargo's evil twin to work with projects in the twilight zone of WSL2_

</div><br />

## Motivation

I have written a blog post about it and even gave a simple solution via Makefile, [go check it out].

The gist of the issue is the following:

> You work with both Windows and WSL2.
> Your repositories live on a NTFS partition.
> Therefore the compilation performance within WSL2 will suffer,
> because the files have to cross the VM/file system boundaries.

## Solution

One approach is to copy the files into a location within WSL which is a Linux based filesystem (like ext4) and do the compilation from there. Optionally you need to copy the artifacts back to the origin.

`wargo` does that as a wrapper around cargo:
- copy the project into a Linux location
- run the provided cargo command
- copy back the artifacts

Currently it does this in a very simple and naive way; workspaces should work out of the box, but mostly I use single package projects.
Also tweaks with the target folder may or may not work properly, the defaults are usually fine for me anyway.

There are some optional features possible, but current state is pretty complete for my personal use cases.

If you believe there is a feature missing or a tweak necessary, feel free to open a pull request or an issue.

## Usage

### Installation

```sh
cargo install wargo
```

### Wargo.toml (optional)

Add a basic `Wargo.toml` to your project if you want to configure the behaviour.
The wargo wrapper does not accept any CLI arguments on its own, so a config file is the only option for now.

```toml
# Wargo.toml

# this is also the default
dest_base_dir = "~/tmp"
```

The file could be completely empty, but at least `dest_base_dir` is good to specify.
Use either a location in your home dir (`~`) or any other absolute path, which is **not** an NTFS file system.

See a complete and commented example [here].

### Run it

```sh
# instead of `cargo` just replace it with `wargo`:
wargo check
wargo build
wargo build --release
wargo run

# alternatively also callable as a cargo subcommand `wsl`:
cargo wsl build
```

## Safety

This crate uses ``#![forbid(unsafe_code)]`` to ensure everything is implemented in 100% Safe Rust.

## License

<sup>
Licensed under either of
  <a href="https://raw.githubusercontent.com/asaaki/wargo/main/LICENSE-APACHE">Apache License, Version 2.0</a> or
  <a href="https://raw.githubusercontent.com/asaaki/wargo/main/LICENSE-MIT">MIT license</a>
at your option.
</sup>

<br/>

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
</sub>

<!-- links -->

[go check it out]: https://markentier.tech/posts/2022/01/speedy-rust-builds-under-wsl2/
[here]: https://github.com/asaaki/wargo/blob/main/Wargo.toml
