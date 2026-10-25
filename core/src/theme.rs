use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub supersample: f64,
    pub memory_budget_mib: u32,
    pub background_darken: f64,
    pub fit: bool,
    pub gap: u32,
    pub accent: String,
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
            supersample: 2.0,
            memory_budget_mib: 128,
            background_darken: 0.35,
            fit: true,
            gap: 0,
            accent: "#9CBEFF".into(),
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
            &t.accent,
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
        if !t.supersample.is_finite()
            || !(1.0..=4.0).contains(&t.supersample)
            || !(16..=512).contains(&t.memory_budget_mib)
            || !t.background_darken.is_finite()
            || !(0.0..=1.0).contains(&t.background_darken)
        {
            return Err(
                "supersample must be 1..4, memory_budget_mib 16..512, background_darken 0..1"
                    .into(),
            );
        }
        if !(10..=40).contains(&t.font_size)
            || t.rounding > 4
            || t.gap > 32
            || t.font.is_empty()
            || t.font.len() > 128
        {
            return Err(
                "font_size must be 10..40, rounding 0..4, gap 0..32, font nonempty and <=128 bytes"
                    .into(),
            );
        }
        Ok(t)
    }
}
