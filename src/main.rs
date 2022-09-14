use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process,
};

use clap::Parser;
use hashbrown::HashMap;
use serde::Deserialize;

#[derive(Debug, Parser)]
#[clap(version)]
struct Args {
    /// packages directory
    ///
    /// Packages found in this directory will be considered for patching.
    packages: String,

    /// patches
    ///
    /// A file containing patches to be applied
    patches: String,
}

/// patches to be applied to an aircraft's config files
///
/// Patches take the form key / value, where a given key is to be updated to a given value.
#[derive(Debug, Deserialize)]
struct Patch {
    #[serde(default)]
    engines: HashMap<String, String>,
    #[serde(default)]
    flight_model: HashMap<String, String>,
}

impl Patch {
    fn diff(&self, path: impl AsRef<Path>) -> io::Result<Diff> {
        let mut diff = Diff::default();

        if !self.engines.is_empty() {
            if let Some(target) = find_path(path.as_ref(), "engines.cfg") {
                let text = fs::read_to_string(&target)?;
                diff.engines = PathChanges {
                    path: target,
                    changes: build_diff(&self.engines, &text),
                };
            }
        }

        if !self.flight_model.is_empty() {
            if let Some(target) = find_path(path.as_ref(), "flight_model.cfg") {
                let text = fs::read_to_string(&target)?;
                diff.flight_model = PathChanges {
                    path: target,
                    changes: build_diff(&self.flight_model, &text),
                };
            }
        }

        Ok(diff)
    }
}

fn find_path(path: impl AsRef<Path>, filename: &str) -> Option<PathBuf> {
    walkdir::WalkDir::new(path)
        .contents_first(true)
        .into_iter()
        .find_map(|entry| {
            let entry = entry.ok()?;
            entry
                .path()
                .ends_with(filename)
                .then_some(entry.into_path())
        })
}

fn build_diff(patch: &HashMap<String, String>, text: &str) -> HashMap<String, (String, String)> {
    let mut diff = HashMap::new();

    for line in text.lines() {
        if let Some((key, tail)) = line.split_once('=') {
            let key = key.trim();

            // We have no use for comments at this stage, but we'll do this again later and
            // do something with them.

            let (value, _comment) = tail.split_once(';').unwrap_or((tail, ""));
            if let Some(change) = patch.get(key) {
                // If the value is equal to the changed value, we... actually don't want to bother
                // with this.

                if value.trim() == change {
                    continue;
                }

                diff.insert(key.to_owned(), (change.to_owned(), value.to_owned()));
            }
        }
    }

    diff
}

#[derive(Debug, Default)]
struct PathChanges {
    path: PathBuf,
    changes: HashMap<String, (String, String)>,
}

/// diff between a given patch and a given file
///
/// If a patch needs to be applied, there will be keyes in these maps. If the maps are empty, the
/// patch has already been applied or the patch contained nothing.
#[derive(Debug, Default)]
struct Diff {
    engines: PathChanges,
    flight_model: PathChanges,
}

impl Diff {
    fn write_changes(&self) -> io::Result<()> {
        if !self.engines.changes.is_empty() {
            write_modified_file(&self.engines)?;
        }

        if !self.flight_model.changes.is_empty() {
            write_modified_file(&self.flight_model)?;
        }

        Ok(())
    }
}

fn write_modified_file(patch: &PathChanges) -> io::Result<()> {
    let mut buf = Vec::new();
    let text = fs::read_to_string(&patch.path)?;

    for line in text.lines() {
        // If we get a key from this split_once, we need to check to see whether this is a key
        // we want to modify. Otherwise, just write the line to our output buffer without
        // modifications.

        if let Some((key, tail)) = line.split_once('=') {
            let key = key.trim();

            if let Some((value, original)) = patch.changes.get(key) {
                // Because we found a change, we're going to A) write our modified value to output
                // instead of the original value; B) include the original value as a "comment";
                // and C) include the original comment (if applicable) in a second comment.

                match tail.split_once(';') {
                    Some((_, comment)) => {
                        let f = format!("{key} = {value} ; {original} ; {comment}");
                        println!("{f}");
                        writeln!(buf, "{f}")?;
                    }

                    None => {
                        let f = format!("{key} = {value} ; {original}");
                        println!("{f}");
                        writeln!(buf, "{f}")?;
                    }
                }
            } else {
                writeln!(buf, "{line}")?;
            }
        } else {
            writeln!(buf, "{line}")?;
        }
    }

    let backup = patch.path.with_extension("bak.cfg");
    fs::rename(&patch.path, &backup)?;
    fs::write(&patch.path, buf)
}

fn main() {
    if let Err(e) = run(&Args::parse()) {
        eprintln!("{e}");
        process::exit(1);
    }
}

fn run(args: &Args) -> anyhow::Result<()> {
    let patches = read_patches(args.patches.as_ref())?;
    let packages = read_packages(args.packages.as_ref(), &patches)?;

    for (package, patch) in packages {
        let diff = patch.diff(&package)?;
        diff.write_changes()?;
    }

    Ok(())
}

fn read_patches(path: &Path) -> anyhow::Result<HashMap<String, Patch>> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn read_packages<'a>(
    path: &Path,
    patches: &'a HashMap<String, Patch>,
) -> io::Result<impl Iterator<Item = (PathBuf, &'a Patch)> + 'a> {
    let candidates = fs::read_dir(path)?.filter_map(|entry| {
        let entry = entry.ok()?;
        let path = entry.path();
        path.is_dir().then_some(path)
    });

    Ok(candidates.filter_map(|path| {
        let name = path.file_name()?.to_str()?;
        patches.get(name).map(|patch| (path, patch))
    }))
}
