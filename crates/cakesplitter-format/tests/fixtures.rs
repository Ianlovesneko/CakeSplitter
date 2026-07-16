use std::{fs, path::PathBuf};

use cakesplitter_format::CakeManifest;

#[test]
fn valid_fixtures_are_accepted() {
    for fixture in fixture_files("valid") {
        let manifest: CakeManifest = serde_json::from_str(&fs::read_to_string(&fixture).unwrap())
            .unwrap_or_else(|error| panic!("{} did not parse: {error}", fixture.display()));
        manifest
            .validate()
            .unwrap_or_else(|error| panic!("{} did not validate: {error}", fixture.display()));
    }
}

#[test]
fn invalid_fixtures_are_rejected() {
    for fixture in fixture_files("invalid") {
        let result = serde_json::from_str::<CakeManifest>(&fs::read_to_string(&fixture).unwrap())
            .map_err(|error| error.to_string())
            .and_then(|manifest| manifest.validate().map_err(|error| error.to_string()));
        assert!(
            result.is_err(),
            "{} was unexpectedly accepted",
            fixture.display()
        );
    }
}

fn fixture_files(kind: &str) -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join(kind);
    let mut fixtures: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    fixtures.sort();
    fixtures
}
