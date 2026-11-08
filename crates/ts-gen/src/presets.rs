//! Garment presets (`ai/presets/garment_presets.json`): the hidden prompt text
//! for each garment type, plus how to place and skin what comes out.

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Presets {
    pub common: Common,
    pub garments: Vec<Garment>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Common {
    pub text_prompt: String,
    pub edit_prompt: String,
    pub solid_text_prompt: String,
    pub solid_edit_prompt: String,
    /// Only used by models that take a negative prompt (not the fast one).
    #[serde(default)]
    pub negative: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Garment {
    pub id: String,
    pub label: String,
    /// Icon file name (without .png) in `ai/presets/icons`.
    #[serde(default)]
    pub icon: String,
    /// Outfit library category this goes under.
    #[serde(default)]
    pub library_category: String,
    /// What it is, e.g. "a jacket".
    pub item: String,
    /// Words that ask for open sleeves / neck etc.
    #[serde(default)]
    pub openings: String,
    /// Openings the cleanup step should cut (for later use).
    #[serde(default)]
    pub cut_openings: Vec<String>,
    /// A solid object (backpack, accessory), not a hollow garment.
    #[serde(default)]
    pub solid: bool,
    /// "cloth" (copy the body's skin weights) or "rigid" (follows one bone).
    #[serde(default = "cloth")]
    pub skin: String,
    #[serde(default)]
    pub rigid_bone: Option<String>,
    /// Target triangle count for the 3D step.
    #[serde(default = "default_faces")]
    pub faces: u32,
    pub place: Place,
}

fn cloth() -> String {
    "cloth".into()
}

fn default_faces() -> u32 {
    150_000
}

/// Where a new AI garment goes on the body.
#[derive(Clone, Debug, Deserialize)]
pub struct Place {
    /// Which edge of the garment lines up with `anchor_bone`: "top", "bottom" or "center".
    pub anchor: String,
    /// A bone name, or "floor", or "hands" (between the two hands).
    pub anchor_bone: String,
    /// Move up (+) or down (-) from the anchor, in cm.
    #[serde(default)]
    pub offset_cm: f32,
    /// Move behind the body (backpacks), in cm.
    #[serde(default)]
    pub behind_cm: f32,
    /// One or two bones whose spacing sets the garment's width.
    #[serde(default)]
    pub width_bones: Vec<String>,
    /// Where to measure the garment's width: 0 = at the anchor edge, 1 = the far edge.
    #[serde(default = "half")]
    pub width_at: f32,
    /// Garment width = bone spacing × this.
    #[serde(default = "one")]
    pub width_scale: f32,
    /// Fixed target width (cm) instead of bone spacing.
    #[serde(default)]
    pub width_cm: Option<f32>,
    /// Extra turn around the up axis, in degrees.
    #[serde(default)]
    pub turn_deg: f32,
    /// A left/right pair (shoes, gloves): the two halves go to these two bones.
    #[serde(default)]
    pub pair: Vec<String>,
}

fn half() -> f32 {
    0.5
}

fn one() -> f32 {
    1.0
}

impl Presets {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Couldn't read {}", path.display()))?;
        let presets: Presets = serde_json::from_str(&text)
            .with_context(|| format!("{} has a mistake in it", path.display()))?;
        anyhow::ensure!(!presets.garments.is_empty(), "No garments in {}", path.display());
        Ok(presets)
    }

    pub fn find(&self, id: &str) -> Option<&Garment> {
        self.garments.iter().find(|g| g.id == id)
    }

    /// The full (hidden) prompt for a garment type and what the user typed.
    /// `with_image`: the user gave a reference picture to work from.
    pub fn prompt(&self, garment: &Garment, user_text: &str, with_image: bool) -> String {
        let template = match (garment.solid, with_image) {
            (false, false) => &self.common.text_prompt,
            (false, true) => &self.common.edit_prompt,
            (true, false) => &self.common.solid_text_prompt,
            (true, true) => &self.common.solid_edit_prompt,
        };
        let user = user_text.trim().trim_end_matches(['.', ',', ' ']);
        let user = if user.is_empty() {
            String::new()
        } else {
            format!(", {user}")
        };
        let text = template
            .replace("{item}", &garment.item)
            .replace("{user}", &user)
            .replace("{openings}", garment.openings.trim().trim_end_matches('.'));
        tidy(&text)
    }
}

/// Remove empty sentences left by blank parts (". ." → ".").
fn tidy(text: &str) -> String {
    let mut out = text.to_string();
    for (from, to) in [(". .", "."), ("..", "."), ("  ", " "), (" .", ".")] {
        while out.contains(from) {
            out = out.replace(from, to);
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presets() -> Presets {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ai/presets/garment_presets.json");
        Presets::load(&path).unwrap()
    }

    #[test]
    fn loads_all_types() {
        let p = presets();
        assert_eq!(p.garments.len(), 13);
        for g in &p.garments {
            assert!(!g.label.is_empty());
            assert!(g.skin == "cloth" || g.skin == "rigid", "{}", g.id);
        }
    }

    #[test]
    fn builds_prompts() {
        let p = presets();
        let jacket = p.find("jackets").unwrap();
        let text = p.prompt(jacket, "worn brown leather, fleece collar.", false);
        assert!(text.starts_with("Ghost mannequin product photograph of a jacket, worn brown leather, fleece collar."));
        assert!(text.contains("sleeve cuffs"));
        assert!(!text.contains('{'));
        let bag = p.find("backpacks").unwrap();
        let text = p.prompt(bag, "", true);
        assert!(!text.contains(". ."), "{text}");
        assert!(text.contains("image 1"));
    }
}
