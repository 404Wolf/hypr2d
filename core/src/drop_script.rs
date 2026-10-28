//! Public Lua operations only. Never exchange targets between layout spaces.
use crate::{ffi, guarded};
// Numeric Lua selectors address monitor list positions, not native IDs.
pub(crate) const MONITOR_LOOKUP:&str="local function monitor_by_id(id) for _,m in ipairs(hl.get_monitors()) do if m.id==id then return m end end error('target display missing') end; ";
pub(crate) fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() && (c as u32) <= 127 => out.push_str(&format!("\\{:03}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}
pub fn script(drop: &ffi::UIDrop, source: &str, x: f64, y: f64) -> String {
    guarded(|| {
        if drop.whole_group {
            return whole_group_script(drop, source, x, y);
        }
        let target = if drop.stack.is_empty() {
            &drop.swap
        } else {
            &drop.stack
        };
        let w = quote(&format!("address:{}", drop.window));
        let t = quote(&format!("address:{target}"));
        let dest = quote(&drop.workspace);
        let mut code=MONITOR_LOOKUP.to_owned()+&format!("local w=assert(hl.get_window({w}),'source window not found'); assert(w.mapped,'source window unmapped'); local function dispatch(d) local result=hl.dispatch(d); assert(result.ok,result.error) end; ");
        if !target.is_empty() {
            code.push_str(&format!(
                "local t=assert(hl.get_window({t}),'target window not found'); assert(t.mapped,'target window unmapped'); "
            ));
        }
        code.push_str("local saved={}; local focus=hl.get_active_monitor(); for _,m in ipairs(hl.get_monitors()) do saved[#saved+1]={monitor=m,workspace=m.active_workspace,special=m.active_special_workspace} end; local success,failure=pcall(function() ");
        code.push_str(&format!(
            "dispatch(hl.dsp.focus({{monitor=monitor_by_id({})}})); ",
            drop.monitor
        ));
        code.push_str(
            "local function detach(win) local g=win.group; if not g then return end; local successor=nil; if g.current==win then local members=g.members; for i,member in ipairs(members) do if member==win and #members>1 then successor=members[i % #members+1]; break end end end; dispatch(hl.dsp.window.move({window=win,out_of_group=true})); if successor and successor.group and successor.group.size>1 and successor.group.current~=successor then for i,member in ipairs(successor.group.members) do if member==successor then dispatch(hl.dsp.group.active({window=successor,index=i})); break end end end end; local function move(win,ws) dispatch(hl.dsp.window.move({window=win,workspace=ws,follow=false})); detach(win) end; detach(w); ",
        );
        if !drop.stack.is_empty() {
            code.push_str(&format!("dispatch(hl.dsp.window.move({{window=w,workspace={dest},follow=false}})); if not t.group then dispatch(hl.dsp.group.toggle({{window=t}})) end; assert(t.group,'target group unavailable'); t.group:add(w); dispatch(hl.dsp.group.active({{window=w,index=t.group.current_index}})); "));
        } else if !drop.swap.is_empty() {
            code.push_str("detach(t); ");
            if source == drop.workspace {
                code.push_str("local wi,wc,ti,tc=w.fullscreen,w.fullscreen_client,t.fullscreen,t.fullscreen_client; dispatch(hl.dsp.window.fullscreen_state({window=w,internal=0,client=0})); dispatch(hl.dsp.window.fullscreen_state({window=t,internal=0,client=0})); dispatch(hl.dsp.window.swap({window=w,target=t})); dispatch(hl.dsp.window.fullscreen_state({window=w,internal=wi,client=wc})); dispatch(hl.dsp.window.fullscreen_state({window=t,internal=ti,client=tc})); ");
            } else {
                code.push_str(&format!("move(w,{dest}); move(t,{}); ", quote(source)));
            }
        } else {
            code.push_str(&format!("move(w,{dest}); "));
            if drop.floating {
                code.push_str(&format!(
                    "dispatch(hl.dsp.window.move({{window=w,x={x},y={y},relative=false}})); "
                ));
            }
        }
        code.push_str(&format!("end); for _,s in ipairs(saved) do local m=s.monitor; if m and m.enabled then if m.active_workspace~=s.workspace and s.workspace then dispatch(hl.dsp.focus({{monitor=m}})); dispatch(hl.dsp.focus({{workspace=s.workspace}})) end; local wanted=s.special; if wanted and wanted.is_empty and wanted.addressable_name=={} then wanted=nil end; if m.active_special_workspace~=wanted then dispatch(hl.dsp.focus({{monitor=m}})); if m.active_special_workspace then dispatch(hl.dsp.workspace.toggle_special(m.active_special_workspace.addressable_name:sub(9))) end; if wanted then dispatch(hl.dsp.workspace.toggle_special(wanted.addressable_name:sub(9))) end end end end; if focus then dispatch(hl.dsp.focus({{monitor=focus}})) end; if not success then error(failure) end; ",quote(source)));
        code
    })
}
fn whole_group_script(drop: &ffi::UIDrop, source: &str, x: f64, y: f64) -> String {
    let w = quote(&format!("address:{}", drop.window));
    let dest = quote(&drop.workspace);
    let src = quote(source);
    let mut s=MONITOR_LOOKUP.to_owned()+&format!("local function dispatch(d) local r=hl.dispatch(d); assert(r.ok,r.error) end; local w=assert(hl.get_window({w}),'source missing'); assert(w.mapped,'source unmapped'); ");
    if !drop.swap.is_empty() {
        s+=&format!("local t=assert(hl.get_window({}),'target missing'); assert(t.mapped,'target unmapped'); ",quote(&format!("address:{}",drop.swap)));
    }
    s+="local saved={};local focused=hl.get_active_monitor(); for _,m in ipairs(hl.get_monitors()) do saved[#saved+1]={monitor=m,workspace=m.active_workspace,special=m.active_special_workspace} end; local success,failure=pcall(function() dispatch(hl.dsp.group.lock({action='set'})); ";
    s += &format!(
        "dispatch(hl.dsp.focus({{monitor=monitor_by_id({})}})); ",
        drop.monitor
    );
    if !drop.swap.is_empty() && source == drop.workspace {
        s += "dispatch(hl.dsp.window.swap({window=w,target=t})); ";
    } else {
        s += &format!("dispatch(hl.dsp.window.move({{window=w,workspace={dest},follow=false}})); ");
        if !drop.swap.is_empty() {
            s += &format!(
                "dispatch(hl.dsp.window.move({{window=t,workspace={src},follow=false}})); "
            );
        }
    }
    if drop.floating {
        s += &format!("dispatch(hl.dsp.window.move({{window=w,x={x},y={y},relative=false}})); ");
    }
    s+="end); dispatch(hl.dsp.group.lock({action='unset'})); for _,s in ipairs(saved) do local m=s.monitor; if m and m.enabled then dispatch(hl.dsp.focus({monitor=m})); if s.workspace then dispatch(hl.dsp.focus({workspace=s.workspace})) end; local wanted=s.special;if wanted and wanted.is_empty then wanted=nil end;if m.active_special_workspace~=wanted then if m.active_special_workspace then dispatch(hl.dsp.workspace.toggle_special(m.active_special_workspace.addressable_name:sub(9))) end;if wanted then dispatch(hl.dsp.workspace.toggle_special(wanted.addressable_name:sub(9))) end end end end; if focused then dispatch(hl.dsp.focus({monitor=focused})) end; if not success then error(failure) end; ";
    s
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shift_preserves_group_and_alt_remains_an_exchange() {
        let mut d = ffi::UIDrop {
            window: "0xa".into(),
            workspace: "21".into(),
            whole_group: true,
            ..Default::default()
        };
        let s = script(&d, "11", 0., 0.);
        assert!(s.contains("assert(hl.get_window(\"address:0xa\")"));
        assert!(!s.contains("out_of_group"));
        assert!(s.contains("group.lock({action='set'})"));
        assert!(s.contains("group.lock({action='unset'})"));
        assert_eq!(s.matches("window.move({window=w,workspace=").count(), 1);
        assert!(s.contains("monitor_by_id(0)"));
        assert!(s.contains("m.id==id"));
        d.swap = "0xb".into();
        let s = script(&d, "11", 0., 0.);
        assert!(s.contains("assert(hl.get_window(\"address:0xb\")"));
        assert!(s.contains("window.move({window=t,workspace=\"11\""));
        assert!(!s.contains("out_of_group"));
        d.whole_group = false;
        let s = script(&d, "11", 0., 0.);
        assert!(s.contains("out_of_group=true"));
    }
    #[test]
    fn special_swap_uses_two_silent_dispatches_and_quotes_names() {
        let d = ffi::UIDrop {
            window: "0xa".into(),
            swap: "0xb".into(),
            workspace: "special:a\"\\\n".into(),
            ..Default::default()
        };
        let s = script(&d, "21", 0., 0.);
        assert!(s.contains("move(w,") && s.contains("move(t,\"21\")"));
        assert!(!s.contains("window.swap"));
        assert!(s.contains("assert(hl.get_window(\"address:0xa\")"));
        assert!(s.contains("assert(hl.get_window(\"address:0xb\")"));
        assert!(!s.contains("window=nil"));
        assert!(
            s.contains("successor.group.size>1"),
            "singleton selection is already current and its dispatcher rejects it"
        );
        assert!(s.contains("detach(t)"));
        assert!(s.contains("special:a\\\"\\\\\\010"));
        assert_eq!(quote("λ"), "\"λ\"");
    }
}
