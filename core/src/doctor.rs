//! Diagnose native layout ownership; plan only unambiguous recovery operations.
use crate::{drop_script::quote, ffi, guarded};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub struct Analysis {
    pub errors: Vec<String>,
    warnings: BTreeSet<String>,
    blocked: Vec<String>,
    rebuild: BTreeSet<String>,
    dissolve: BTreeSet<String>,
}
impl Default for ffi::DoctorPlan {
    fn default() -> Self {
        Self {
            rebuild: vec![],
            dissolve: vec![],
            error: "Rust repair planning failed".into(),
        }
    }
}
pub fn analyze(workspaces: &[ffi::DoctorWorkspace], windows: &[ffi::DoctorWindow]) -> Analysis {
    let mut a = Analysis {
        errors: vec![],
        warnings: BTreeSet::new(),
        blocked: vec![],
        rebuild: BTreeSet::new(),
        dissolve: BTreeSet::new(),
    };
    let by_id: BTreeMap<_, _> = windows.iter().map(|w| (w.id.as_str(), w)).collect();
    let ws: BTreeMap<_, _> = workspaces.iter().map(|w| (w.key.as_str(), w)).collect();
    let known: BTreeSet<_> = windows.iter().map(|w| w.target.as_str()).collect();
    let mut membership = BTreeMap::<&str, Vec<&str>>::new();
    if by_id.len() != windows.len() {
        a.blocked.push("duplicate mapped window identity".into());
    }
    for space in workspaces {
        if space.reported as usize != space.targets.len() {
            a.errors.push(format!(
                "{}: reported count differs from layout targets",
                space.key
            ));
        }
        let unique: BTreeSet<_> = space.targets.iter().collect();
        if unique.len() != space.targets.len() {
            a.blocked.push(format!("{}: duplicate targets within one algorithm; refusing ambiguous algorithm reconstruction", space.key));
        }
        for target in &space.targets {
            membership.entry(target).or_default().push(&space.key);
            if !known.contains(target.as_str()) {
                a.blocked.push(format!(
                    "{}: unknown/stale layout target {target}",
                    space.key
                ));
            }
        }
    }
    for w in windows {
        if !ws.get(w.workspace.as_str()).is_some_and(|s| s.usable) || w.target.is_empty() {
            a.blocked.push(format!(
                "{}: no authoritative enabled workspace/space or native target",
                w.id
            ));
        }
        let bad_space = w.workspace != w.target_workspace || !w.target_space;
        let bad_ghost = w.workspace != w.ghost_workspace || !w.ghost_space;
        let bad_membership = membership.get(w.target.as_str()).map(Vec::as_slice)
            != Some(&[w.workspace.as_str()][..]);
        if bad_space {
            a.errors.push(format!(
                "{}: window workspace {} differs from target space {}",
                w.id, w.workspace, w.target_workspace
            ));
        }
        if bad_ghost {
            a.errors.push(format!(
                "{}: member/window target ghost space {} differs from workspace {}",
                w.id, w.ghost_workspace, w.workspace
            ));
        }
        if bad_membership {
            a.errors.push(format!(
                "{}: missing, duplicate or wrong-workspace layout target {}",
                w.id, w.target
            ));
        }
        if bad_space || bad_ghost || bad_membership {
            a.rebuild.insert(if w.group.is_empty() {
                w.id.clone()
            } else {
                w.current.clone()
            });
        }
        if !w.group.is_empty() {
            let members: BTreeSet<_> = w.members.iter().collect();
            if members.len() != w.members.len()
                || !members.contains(&w.id)
                || !members.contains(&w.current)
            {
                a.blocked
                    .push(format!("{}: invalid group membership/current", w.id));
            }
            for member in &w.members {
                if !by_id.get(member.as_str()).is_some_and(|other| {
                    other.group == w.group
                        && other.workspace == w.workspace
                        && other.members == w.members
                        && other.current == w.current
                        && other.target == w.target
                }) {
                    a.blocked
                        .push(format!("{}: inconsistent group member {member}", w.id));
                }
            }
            if w.members.len() == 1 {
                a.warnings.insert(format!(
                    "{}: singleton group {}{}",
                    w.id,
                    w.group,
                    if w.locked { " (locked; retained)" } else { "" }
                ));
                if !w.locked {
                    a.dissolve.insert(w.id.clone());
                }
            }
        }
        if windows.iter().any(|other| {
            other.id != w.id
                && other.target == w.target
                && (w.group.is_empty() || other.group != w.group)
        }) {
            a.blocked
                .push(format!("{}: native target shared outside one group", w.id));
        }
    }
    a.errors.extend(a.blocked.clone());
    a
}
pub fn report(workspaces: &[ffi::DoctorWorkspace], windows: &[ffi::DoctorWindow]) -> String {
    guarded(|| {
        let a = analyze(workspaces, windows);
        let details: Vec<_> = windows.iter().map(|w| json!({"window":w.id,"workspace":w.workspace,"layout_workspace":w.target_workspace,"member_workspace":w.ghost_workspace,"layout_space_exists":w.target_space,"member_space_exists":w.ghost_space})).collect();
        json!({"ok":a.errors.is_empty(),"errors":a.errors,"warnings":a.warnings,"repairable":a.blocked.is_empty(),"refused":a.blocked,"windows":windows.len(),"workspaces":workspaces.len(),"details":details}).to_string()
    })
}
pub fn plan(workspaces: &[ffi::DoctorWorkspace], windows: &[ffi::DoctorWindow]) -> ffi::DoctorPlan {
    let a = analyze(workspaces, windows);
    ffi::DoctorPlan {
        rebuild: a.rebuild.into_iter().collect(),
        dissolve: a.dissolve.into_iter().collect(),
        error: a.blocked.join("; "),
    }
}
pub fn repair_script(windows: &[String]) -> String {
    let mut script = String::from(
        "local function dispatch(d) local r=hl.dispatch(d); assert(r.ok,r.error) end; ",
    );
    // Resolve every address before any dispatcher. A stale selector must never
    // become nil and invoke Hyprland's implicit focused-window fallback.
    for (i, id) in windows.iter().enumerate() {
        script.push_str(&format!("local w{i}=assert(hl.get_window({}),'repair window not found'); assert(w{i}.mapped,'repair window unmapped'); ",quote(&format!("address:{id}"))));
    }
    script.push_str("local saved={}; local focus=hl.get_active_monitor(); local active=hl.get_active_window(); for _,m in ipairs(hl.get_monitors()) do saved[#saved+1]={monitor=m,workspace=m.active_workspace,special=m.active_special_workspace} end; local success,failure=pcall(function() ");
    for i in 0..windows.len() {
        script.push_str(&format!("if w{i}.group then assert(w{i}.group.size==1,'repair group changed'); dispatch(hl.dsp.window.move({{window=w{i},out_of_group=true}})) end; "));
    }
    script.push_str("end); for _,s in ipairs(saved) do local m=s.monitor; if m and m.enabled then if s.workspace and m.active_workspace~=s.workspace then dispatch(hl.dsp.focus({monitor=m})); dispatch(hl.dsp.focus({workspace=s.workspace})) end; if m.active_special_workspace~=s.special then dispatch(hl.dsp.focus({monitor=m})); if m.active_special_workspace then dispatch(hl.dsp.workspace.toggle_special(m.active_special_workspace.addressable_name:sub(9))) end; if s.special then dispatch(hl.dsp.workspace.toggle_special(s.special.addressable_name:sub(9))) end end end end; if focus then dispatch(hl.dsp.focus({monitor=focus})) end; if active and active.mapped then dispatch(hl.dsp.focus({window=active})) end; if not success then error(failure) end; ");
    script
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (ffi::DoctorWorkspace, ffi::DoctorWindow) {
        (
            ffi::DoctorWorkspace {
                key: "special:3".into(),
                reported: 0,
                usable: true,
                targets: vec![],
            },
            ffi::DoctorWindow {
                id: "0xa".into(),
                workspace: "special:3".into(),
                target_workspace: String::new(),
                ghost_workspace: "special:3".into(),
                target: "0xt".into(),
                target_space: true,
                ghost_space: true,
                locked: false,
                group: "g".into(),
                members: vec!["0xa".into()],
                current: "0xa".into(),
            },
        )
    }
    #[test]
    fn orphaned_group_space_is_rebuilt_before_singleton_detach() {
        let (mut ws, mut w) = fixture();
        let p = plan(std::slice::from_ref(&ws), std::slice::from_ref(&w));
        assert!(p.error.is_empty());
        assert_eq!(p.rebuild, ["0xa"]);
        assert_eq!(p.dissolve, ["0xa"]);
        let bad: serde_json::Value =
            serde_json::from_str(&report(std::slice::from_ref(&ws), std::slice::from_ref(&w)))
                .unwrap();
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["details"][0]["layout_space_exists"], true);
        ws.reported = 1;
        ws.targets.push("0xt".into());
        w.target_workspace = w.workspace.clone();
        assert!(analyze(std::slice::from_ref(&ws), std::slice::from_ref(&w))
            .errors
            .is_empty());
        w.ghost_workspace.clear();
        assert_eq!(
            plan(std::slice::from_ref(&ws), std::slice::from_ref(&w)).rebuild,
            ["0xa"]
        );
        w.locked = true;
        assert!(plan(std::slice::from_ref(&ws), std::slice::from_ref(&w))
            .dissolve
            .is_empty());
        ws.usable = false;
        assert!(!plan(&[ws], &[w]).error.is_empty());
    }
    #[test]
    fn refuses_unknown_targets_and_ambiguous_groups_and_asserts_all_lookups_first() {
        let (mut ws, mut w) = fixture();
        ws.targets.push("unknown".into());
        assert!(!plan(std::slice::from_ref(&ws), std::slice::from_ref(&w))
            .error
            .is_empty());
        ws.targets = vec![w.target.clone(), w.target.clone()];
        assert!(!plan(std::slice::from_ref(&ws), std::slice::from_ref(&w))
            .error
            .is_empty());
        ws.targets.clear();
        w.members.push("missing".into());
        assert!(!plan(&[ws], &[w]).error.is_empty());
        let s = repair_script(&["0xa".into(), "0xb".into()]);
        assert!(s.contains("assert(hl.get_window(\"address:0xa\")"));
        assert!(
            s.find("assert(hl.get_window(\"address:0xb\")").unwrap()
                < s.find("dispatch(hl.dsp.").unwrap()
        );
    }
}
