use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub fit: bool,
    pub background: String,
    pub cell: String,
    pub border: String,
    pub hover: String,
    pub current: String,
    pub target: String,
    pub label: String,
    pub font: String,
    pub font_size: u32,
    pub rounding: u32,
}
impl Default for Theme {
    fn default() -> Self {
        Self {
            fit: true,
            background: "#000000".into(),
            cell: "#303030".into(),
            border: "#353535".into(),
            hover: "#4A4A4A".into(),
            current: "#82AAFF".into(),
            target: "#FFCB6B".into(),
            label: "#EEFFFF".into(),
            font: "sans-serif".into(),
            font_size: 16,
            rounding: 3,
        }
    }
}
impl Theme {
    pub fn parse(s: &str) -> Result<Self, String> {
        let t: Self = serde_json::from_str(s).map_err(|e| e.to_string())?;
        for c in [
            &t.background,
            &t.cell,
            &t.border,
            &t.hover,
            &t.current,
            &t.target,
            &t.label,
        ] {
            if c.len() != 7 || !c.starts_with('#') || !c[1..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("colours must be #RRGGBB".into());
            }
        }
        if !(10..=40).contains(&t.font_size)
            || t.rounding > 4
            || t.font.is_empty()
            || t.font.len() > 128
        {
            return Err(
                "font_size must be 10..40, rounding 0..4, font nonempty and <=128 bytes".into(),
            );
        }
        Ok(t)
    }
}
