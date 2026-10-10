use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// A font family installed on the system, as offered in the text font picker.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontFamily {
    pub name: String,
    /// Other names of the same faces (legacy family names); the webview may only know one of them.
    pub aliases: Vec<String>,
    pub weights: Vec<u16>,
    pub italic: bool,
}

#[derive(Default)]
struct Group {
    aliases: BTreeSet<String>,
    weights: BTreeSet<u16>,
    italic: bool,
    latin: bool,
}

const NAME_FAMILY: u16 = 1;
const NAME_TYPOGRAPHIC_FAMILY: u16 = 16;

/// Scans installed fonts once per run; later calls return the cached list.
pub fn system_families() -> Vec<FontFamily> {
    static CACHE: OnceLock<Vec<FontFamily>> = OnceLock::new();
    CACHE.get_or_init(scan).clone()
}

fn scan() -> Vec<FontFamily> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for face in db.faces() {
        let Some(name) = face.families.first().map(|(name, _)| name.trim().to_string()) else { continue };
        if name.is_empty() || name.starts_with('@') || name.starts_with('.') {
            continue;
        }
        let names = db.with_face_data(face.id, face_names).unwrap_or_default();
        let group = groups.entry(name.clone()).or_default();
        group.weights.insert(face.weight.0);
        group.italic |= face.style != fontdb::Style::Normal;
        group.latin |= names.latin;
        group.aliases.extend(names.families.into_iter().filter(|alias| alias != &name && !alias.starts_with('@')));
    }
    groups
        .into_iter()
        .filter(|(_, group)| group.latin)
        .map(|(name, group)| FontFamily {
            name,
            aliases: group.aliases.into_iter().collect(),
            weights: group.weights.into_iter().collect(),
            italic: group.italic,
        })
        .collect()
}

#[derive(Default)]
struct FaceNames {
    families: Vec<String>,
    latin: bool,
}

fn face_names(data: &[u8], index: u32) -> FaceNames {
    let Ok(face) = ttf_parser::Face::parse(data, index) else { return FaceNames::default() };
    let latin = ['A', 'a', 'Z', 'z', '0'].iter().all(|c| face.glyph_index(*c).is_some());
    let families = face
        .names()
        .into_iter()
        .filter(|name| (name.name_id == NAME_FAMILY || name.name_id == NAME_TYPOGRAPHIC_FAMILY) && name.is_unicode() && name.language_id == 0x0409)
        .filter_map(|name| name.to_string())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    FaceNames { families, latin }
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_installed_fonts() {
        let families = super::system_families();
        if cfg!(windows) {
            let arial = families.iter().find(|family| family.name == "Arial").expect("Arial is installed on Windows");
            assert!(arial.weights.contains(&400) && arial.weights.contains(&700));
            assert!(!families.iter().any(|family| family.name == "Wingdings"), "symbol fonts are skipped");
        }
    }
}
