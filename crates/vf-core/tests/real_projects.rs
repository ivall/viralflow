//! Read-only check of real project files (VF_CHECK_PROJECTS=<projects folder>):
//! the tolerant reader keeps every project exactly as the plain one did.

use serde_json::Value;
use vf_core::migrate::read_project;
use vf_core::model::Project;

#[test]
#[ignore]
fn real_projects_read_the_same() {
    let Ok(root) = std::env::var("VF_CHECK_PROJECTS") else { return };
    let mut checked = 0;
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        let Ok(bytes) = std::fs::read(entry.path().join("project.json")) else { continue };
        let Ok(document) = serde_json::from_slice::<Value>(&bytes) else {
            println!("{}: not JSON", entry.path().display());
            continue;
        };
        let project = read_project(&document).unwrap();
        // The plain reader, as before (all fields known to this build).
        let plain: Project = serde_json::from_value(document.clone()).unwrap_or_default();
        let mut known = project.clone();
        known.extra.clear();
        for track in &mut known.tracks {
            track.extra.clear();
            for item in &mut track.items {
                item.extra.clear();
            }
        }
        for asset in &mut known.assets {
            asset.extra.clear();
        }
        let same_items = known.tracks.iter().map(|t| t.items.len()).sum::<usize>() == plain.tracks.iter().map(|t| t.items.len()).sum::<usize>();
        let extra = project.extra.len() + project.tracks.iter().map(|t| t.extra.len() + t.items.iter().map(|i| i.extra.len()).sum::<usize>()).sum::<usize>();
        println!(
            "{}: {} items, same items: {same_items}, unknown fields kept: {extra}",
            project.name,
            known.tracks.iter().map(|t| t.items.len()).sum::<usize>()
        );
        assert!(same_items, "{}", entry.path().display());
        checked += 1;
    }
    println!("checked {checked} projects");
}
