use std::{
    env,
    ffi::{CString, OsString},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "kitdoom",
    about = "Play Doom in a Kitty graphics terminal",
    trailing_var_arg = true
)]
pub struct Args {
    #[arg(long, value_name = "DIR")]
    pub asset_dir: Option<PathBuf>,

    #[arg(
        value_name = "DOOM_ARG",
        num_args = 0..,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    pub doom_args: Vec<OsString>,
}

pub fn resolve_asset_dir(asset_dir: Option<PathBuf>) -> Result<PathBuf> {
    let candidate = match asset_dir {
        Some(path) => path,
        None => env::var_os("KITDOOM_ASSET_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                let local = PathBuf::from("assets");
                local.exists().then_some(local)
            })
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")),
    };

    if !candidate.exists() {
        bail!("asset directory does not exist: {}", candidate.display());
    }

    candidate
        .canonicalize()
        .with_context(|| format!("failed to resolve asset directory {}", candidate.display()))
}

pub fn build_doom_argv(asset_dir: &Path, doom_args: Vec<OsString>) -> Vec<OsString> {
    let mut argv = Vec::with_capacity(doom_args.len() + 3);
    argv.push(
        env::args_os()
            .next()
            .unwrap_or_else(|| OsString::from("kitdoom")),
    );

    if !has_iwad_arg(&doom_args) {
        argv.push(OsString::from("-iwad"));
        argv.push(asset_dir.join("doom1.wad").into_os_string());
    }

    argv.extend(doom_args);
    argv
}

pub fn to_c_args(args: &[OsString]) -> Result<Vec<CString>> {
    args.iter()
        .map(|arg| {
            CString::new(arg.to_string_lossy().into_owned())
                .with_context(|| format!("argument contains an interior NUL byte: {arg:?}"))
        })
        .collect()
}

fn has_iwad_arg(args: &[OsString]) -> bool {
    args.iter()
        .any(|arg| arg.to_string_lossy().eq_ignore_ascii_case("-iwad"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_default_iwad_when_missing() {
        let asset_dir = PathBuf::from("/tmp/assets");
        let argv = build_doom_argv(&asset_dir, vec![OsString::from("-nosound")]);

        assert!(argv.iter().any(|arg| arg == "-iwad"));
        assert!(argv.iter().any(|arg| arg == "/tmp/assets/doom1.wad"));
        assert!(argv.iter().any(|arg| arg == "-nosound"));
    }

    #[test]
    fn preserves_explicit_iwad() {
        let asset_dir = PathBuf::from("/tmp/assets");
        let argv = build_doom_argv(
            &asset_dir,
            vec![OsString::from("-iwad"), OsString::from("/games/doom.wad")],
        );

        assert_eq!(argv.iter().filter(|arg| *arg == "-iwad").count(), 1);
        assert!(argv.iter().any(|arg| arg == "/games/doom.wad"));
        assert!(!argv.iter().any(|arg| arg == "/tmp/assets/doom1.wad"));
    }
}
