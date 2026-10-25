use crate::{ffi, guarded};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub fn report(workspaces: &[ffi::DoctorWorkspace], windows: &[ffi::DoctorWindow]) -> String {
    guarded(|| {
        let mut errors = Vec::new();
        let mut warnings = BTreeSet::new();
        let by_id: BTreeMap<_, _> = windows.iter().map(|w| (w.id.as_str(), w)).collect();
        let ws: BTreeSet<_> = workspaces.iter().map(|w| w.key.as_str()).collect();
        let mut membership = BTreeMap::<&str, Vec<&str>>::new();
        for space in workspaces {
            if space.reported as usize != space.targets.len() {
                errors.push(format!(
                    "{}: reported count differs from layout targets",
                    space.key
                ));
            }
            for target in &space.targets {
                membership.entry(target).or_default().push(&space.key);
                if !by_id.contains_key(target.as_str()) {
                    errors.push(format!("{}: stale target {target}", space.key));
                }
            }
        }
        for w in windows {
            if !ws.contains(w.workspace.as_str()) || w.workspace != w.target_workspace {
                errors.push(format!(
                    "{}: window workspace {} differs from target space {}",
                    w.id, w.workspace, w.target_workspace
                ));
            }
            let representative = if w.group.is_empty() {
                &w.id
            } else {
                &w.current
            };
            if membership.get(representative.as_str()).map(Vec::as_slice)
                != Some(&[w.workspace.as_str()][..])
            {
                errors.push(format!(
                    "{}: missing, duplicate or wrong-workspace layout target {representative}",
                    w.id
                ));
            }
            if !w.group.is_empty() {
                if w.members.len() == 1 {
                    warnings.insert(format!("{}: singleton group {}", w.id, w.group));
                }
                let members: BTreeSet<_> = w.members.iter().collect();
                if members.len() != w.members.len()
                    || !members.contains(&w.id)
                    || !members.contains(&w.current)
                {
                    errors.push(format!("{}: invalid group membership/current", w.id));
                }
                for member in &w.members {
                    if !by_id.get(member.as_str()).is_some_and(|other| {
                        other.group == w.group
                            && other.workspace == w.workspace
                            && other.members == w.members
                            && other.current == w.current
                    }) {
                        errors.push(format!("{}: inconsistent group member {member}", w.id));
                    }
                }
            }
        }
        json!({"ok":errors.is_empty(),"errors":errors,"warnings":warnings,"windows":windows.len(),"workspaces":workspaces.len()}).to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_special_membership_without_target_and_reports_singleton() {
        let mut w = ffi::DoctorWindow {
            id: "a".into(),
            workspace: "special:3".into(),
            target_workspace: "special:3".into(),
            group: "g".into(),
            members: vec!["a".into()],
            current: "a".into(),
        };
        let mut space = ffi::DoctorWorkspace {
            key: "special:3".into(),
            reported: 0,
            targets: vec![],
        };
        let bad: serde_json::Value = serde_json::from_str(&report(&[space], &[w])).unwrap();
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["warnings"].as_array().unwrap().len(), 1);
        space = ffi::DoctorWorkspace {
            key: "special:3".into(),
            reported: 1,
            targets: vec!["a".into()],
        };
        w = ffi::DoctorWindow {
            id: "a".into(),
            workspace: "special:3".into(),
            target_workspace: "special:3".into(),
            group: String::new(),
            members: vec![],
            current: String::new(),
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&report(&[space], &[w])).unwrap()["ok"],
            true
        );
    }
}
