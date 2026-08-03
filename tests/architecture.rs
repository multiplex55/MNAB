use std::{fs, path::Path};

fn assert_tree_omits(path: &Path, forbidden: &[&str]) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            assert_tree_omits(&entry.unwrap().path(), forbidden);
        }
    } else if path.extension().is_some_and(|extension| extension == "rs") {
        let source = fs::read_to_string(path).unwrap();
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "{} contains forbidden import {needle}",
                path.display()
            );
        }
    }
}

#[test]
fn dependency_directions_are_preserved() {
    assert_tree_omits(Path::new("src/ui"), &["rusqlite"]);
    assert_tree_omits(Path::new("src/ui.rs"), &["rusqlite"]);
    for path in [
        "src/domain",
        "src/domain.rs",
        "src/calculation",
        "src/calculation.rs",
    ] {
        assert_tree_omits(Path::new(path), &["rusqlite", "egui", "eframe"]);
    }
}
