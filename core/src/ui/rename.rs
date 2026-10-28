use super::*;
impl Overview {
    pub(super) fn rename_key(&mut self, key: u32) -> Option<String> {
        if let Some((target, value)) = &mut self.rename {
            match key {
                0xff1b => {
                    self.rename = None;
                }
                0xff0d => {
                    let action = if target == "grid:new" {
                        format!("@catalog:{}", crate::grid_args("grid create", value, ""))
                    } else {
                        format!("@rename:{}", serde_json::json!([target, value]))
                    };
                    self.rename = None;
                    return Some(action);
                }
                0xff08 => {
                    value.pop();
                }
                97 if self.control => value.clear(),
                _ => {
                    let code = if key & 0xff000000 == 0x01000000 {
                        key & 0xffffff
                    } else {
                        key
                    };
                    if let Some(c) = char::from_u32(code).filter(|c| {
                        !c.is_control() && (key & 0xff000000 == 0x01000000 || code < 0xff00)
                    }) {
                        if value.chars().count() < 96 {
                            value.push(c);
                        }
                    }
                }
            }
            return Some(String::new());
        }
        if key != 114 || self.drag.is_some() || self.region.mode != 0 {
            return None;
        }
        let cell = self.hover.unwrap_or(self.selected);
        if let Some(g) = self
            .mapping
            .chosen()
            .or_else(|| self.shelf_header_at())
            .or_else(|| self.card_at())
        {
            self.rename = Some((
                format!("grid:{}", self.group_slots[g]),
                self.grid_names[g].clone(),
            ));
        } else if let Some(g) = self.header_at() {
            self.rename = Some((self.panel_targets[g].clone(), self.panel_names[g].clone()));
        } else if let Some(c) = self
            .cells
            .get(cell)
            .filter(|c| c.key.starts_with("special:"))
        {
            self.rename = Some((c.key.clone(), c.label.clone()));
        } else if let Some(g) = self
            .cells
            .get(self.selected)
            .map(|c| c.group)
            .filter(|g| *g < self.panel_targets.len())
        {
            self.rename = Some((self.panel_targets[g].clone(), self.panel_names[g].clone()));
        }
        Some(String::new())
    }
    pub(super) fn rename_labels(&self, width: f64, height: f64) -> Vec<ffi::UILabel> {
        self.rename
            .as_ref()
            .map(|(target, value)| ffi::UILabel {
                text: format!(
                    "{}: {value}│   Enter: save · Esc: cancel · Ctrl+A: clear",
                    if target == "grid:new" {
                        "New grid"
                    } else {
                        "Rename"
                    }
                ),
                rect: ffi::UIRect {
                    x: width * 0.15,
                    y: height * 0.45,
                    width: width * 0.7,
                    height: self.font_height + 8.,
                },
                colour: 0,
                band: true,
            })
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_special_name_survives_production_window_refreshes() {
        let mut core = Core::default();
        core.labels.rename("special:8", "chatting", &[]).unwrap();
        let mut ui = core.overview_new(
            &[super::super::tests::output(0, 8934., 6682.)],
            &[ffi::UISpecial {
                monitor: 0,
                name: "special:8".into(),
                shown: false,
            }],
            &[],
            0.,
        );
        ui.overview_windows(&[]);
        assert_eq!(
            ui.cells
                .iter()
                .find(|c| c.key == "special:8")
                .unwrap()
                .label,
            "chatting"
        );
    }
    #[test]
    fn editor_cancels_first_esc_and_quotes_values_without_running_navigation() {
        let core = Core::default();
        let mut ui = core.overview_new(
            &[super::super::tests::output(0, 8934., 6682.)],
            &[],
            &[],
            0.,
        );
        assert_eq!(ui.overview_key(114, false, 200.), "");
        ui.overview_input_modifiers(false, false, true);
        ui.overview_key(97, false, 200.);
        ui.overview_input_modifiers(false, false, false);
        for c in "a \"quoted\" label".chars() {
            ui.overview_key(c as u32, false, 200.);
        }
        let action = ui.overview_key(0xff0d, false, 200.);
        let args: Vec<String> =
            serde_json::from_str(action.strip_prefix("@rename:").unwrap()).unwrap();
        assert_eq!(args, ["monitor:0", "a \"quoted\" label"]);
        ui.overview_key(114, false, 200.);
        ui.overview_key(0xff1b, false, 200.);
        assert!(ui.closing.is_none() && ui.rename.is_none());
    }
}
